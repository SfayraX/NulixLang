//! Владение в проверенном дереве (`spec/core-check.md`, раздел 5): перемещения и копии,
//! частичные перемещения, ветви и циклы, что можно делать с каким местом, один вызов,
//! представления из `match` и условия ветвей.
//!
//! Тело проходится в порядке вычисления (9.27–9.29), и для каждого имени известно, что оно
//! держит: значение целиком, ничего — оно перемещено — или запись, из которой перемещены поля.
//! После ветвей состояния сливаются: место перемещено, если оно перемещено хоть в одной ветви,
//! которая доходит до слияния. Начало цикла — вход, слитый с концом тела и с `continue`; тело
//! проходится, пока это состояние не перестанет меняться, и только затем — с сообщением ошибок.
//! Пока состояние ищется, перемещение с нарушением не выполняется, а запись выполняется: так
//! нарушение не оставляет после себя перемещённых мест, и первой называется сама причина, а не
//! её след в следующем проходе. Каждое новое состояние в начале цикла вбирает прежнее, поэтому
//! поиск кончается.
//!
//! Цикл помнит, что пришло в его начало по обратной дуге, и следующий раз начинает с этого:
//! вложенный цикл не ищет своё состояние заново на каждом проходе внешнего.
//!
//! Попутно у каждого использования места и части временного значения ставится, что с ним
//! делается: копия, перемещение или заимствование (`core::Use`). Это читает перевод в IR.
//!
//! Рекурсия идёт по вложенности дерева, и её ограничивает предел вложенности разборщика.

use std::collections::HashMap;
use std::rc::Rc;

use crate::check::CheckError;
use crate::core::{
    Arg, Block, Body, Expr, ExprKind, FuncId, Local, LocalId, LocalKind, LoopId, Match, Pattern,
    PatternKind, Place, Program, Step, Stmt, StmtKind, TuplePattern, Type, TypeKind, Use,
    VariantValue,
};
use crate::ir::Mode;

type R<T> = Result<T, CheckError>;

/// Проверяет владение в теле и ставит у использований мест, что с ними делается.
pub fn check(p: &Program, body: &mut Body) -> R<()> {
    let Body { locals, block } = body;
    let mut w = Walker::new(p, locals);
    w.block(block)
}

// ───────── состояние ─────────

/// Что держит имя или его поле.
#[derive(Debug, Clone)]
enum Node {
    /// Значение целиком.
    Full,
    /// Ничего: значение перемещено.
    Moved(Moved),
    /// Запись, из которой перемещена часть полей: состояние каждого поля по порядку.
    Parts(Vec<Node>),
}

/// Где место перемещено.
#[derive(Debug, Clone, Copy)]
struct Moved {
    line: u32,
    /// Цикл, в начале прохода которого место пусто только потому, что его переместил прошлый
    /// проход. Сообщение говорит о прошлом проходе, только пока обход внутри этого цикла.
    pass: Option<LoopId>,
}

static FULL: Node = Node::Full;

#[derive(Debug, Clone)]
struct State {
    /// Точка достижима.
    live: bool,
    /// Имена, из которых перемещено всё или часть. Остальные держат значение целиком.
    moved: Moves,
}

impl State {
    fn live() -> State {
        State {
            live: true,
            moved: Moves::default(),
        }
    }

    fn dead() -> State {
        State {
            live: false,
            moved: Moves::default(),
        }
    }

    fn node(&self, id: LocalId) -> &Node {
        self.moved.get(id)
    }
}

/// Разрядов номера имени на уровень дерева `Moves`.
const BITS: u32 = 4;
/// Ветвей у узла дерева `Moves`.
const FAN: usize = 1 << BITS;

#[derive(Debug)]
enum Trie {
    /// Состояние одного имени; `Node::Full` не хранится.
    Leaf(Node),
    /// Ветви по очередному разряду номера, от старшего.
    Inner([Option<Rc<Trie>>; FAN]),
}

/// Состояния имён, где перемещено всё или часть, — постоянное дерево по разрядам номера имени.
/// Копия состояния — одна ссылка, изменение копирует путь к имени, а слияние обходит только
/// те части, которыми два состояния различаются: общие поддеревья у них одни и те же. Так
/// ветвление стоит столько, сколько в ветвях изменилось, а не сколько имён перемещено.
#[derive(Debug, Clone, Default)]
struct Moves {
    /// Разрядов в номере: дерево покрывает номера меньше `FAN^depth`.
    depth: u32,
    root: Option<Rc<Trie>>,
}

impl Moves {
    fn get(&self, id: LocalId) -> &Node {
        if id >> (self.depth * BITS) != 0 {
            return &FULL;
        }
        let mut t = match &self.root {
            Some(t) => t,
            None => return &FULL,
        };
        for level in (0..self.depth).rev() {
            let digit = (id >> (level * BITS)) & (FAN - 1);
            t = match &**t {
                Trie::Inner(children) => match &children[digit] {
                    Some(c) => c,
                    None => return &FULL,
                },
                Trie::Leaf(_) => return &FULL,
            };
        }
        match &**t {
            Trie::Leaf(node) => node,
            Trie::Inner(_) => &FULL,
        }
    }

    /// Дерево покрывает номера меньше `FAN^depth`; прежние номера уходят в ветвь 0.
    fn grow(&mut self, depth: u32) {
        while self.depth < depth {
            if let Some(root) = self.root.take() {
                let mut children: [Option<Rc<Trie>>; FAN] = Default::default();
                children[0] = Some(root);
                self.root = Some(Rc::new(Trie::Inner(children)));
            }
            self.depth += 1;
        }
    }

    fn set(&mut self, id: LocalId, node: Node) {
        let full = matches!(node, Node::Full);
        if full && matches!(self.get(id), Node::Full) {
            return;
        }
        let mut need = self.depth.max(1);
        while id >> (need * BITS) != 0 {
            need += 1;
        }
        self.grow(need);
        self.root = put(self.root.as_ref(), id, self.depth, node);
    }

    fn remove(&mut self, id: LocalId) {
        self.set(id, Node::Full);
    }
}

fn put(t: Option<&Rc<Trie>>, id: LocalId, level: u32, node: Node) -> Option<Rc<Trie>> {
    if level == 0 {
        return match node {
            Node::Full => None,
            node => Some(Rc::new(Trie::Leaf(node))),
        };
    }
    let digit = (id >> ((level - 1) * BITS)) & (FAN - 1);
    let mut children: [Option<Rc<Trie>>; FAN] = match t.map(|t| &**t) {
        Some(Trie::Inner(c)) => c.clone(),
        _ => Default::default(),
    };
    children[digit] = put(children[digit].as_ref(), id, level - 1, node);
    if children.iter().all(Option::is_none) {
        None
    } else {
        Some(Rc::new(Trie::Inner(children)))
    }
}

/// Слияние деревьев одной глубины. `both` — для имени, которое есть в обоих; имя только в
/// `a` остаётся как есть, имя только в `b` проходит через `only_b` (у `None` — как есть).
/// Одно и то же поддерево сливается само с собой в себя.
fn merge_trie(
    a: Option<&Rc<Trie>>,
    b: Option<&Rc<Trie>>,
    both: &dyn Fn(&Node, &Node) -> Node,
    only_b: Option<&dyn Fn(&Node) -> Node>,
) -> Option<Rc<Trie>> {
    match (a, b) {
        (None, None) => None,
        (Some(x), None) => Some(x.clone()),
        (None, Some(y)) => match only_b {
            None => Some(y.clone()),
            Some(f) => map_trie(y, f),
        },
        (Some(x), Some(y)) if Rc::ptr_eq(x, y) => Some(x.clone()),
        (Some(x), Some(y)) => match (&**x, &**y) {
            (Trie::Leaf(p), Trie::Leaf(q)) => Some(Rc::new(Trie::Leaf(both(p, q)))),
            (Trie::Inner(p), Trie::Inner(q)) => {
                let mut children: [Option<Rc<Trie>>; FAN] = Default::default();
                for (k, child) in children.iter_mut().enumerate() {
                    *child = merge_trie(p[k].as_ref(), q[k].as_ref(), both, only_b);
                }
                Some(Rc::new(Trie::Inner(children)))
            }
            // у деревьев одной глубины листья на одном уровне
            _ => Some(x.clone()),
        },
    }
}

fn map_trie(t: &Rc<Trie>, f: &dyn Fn(&Node) -> Node) -> Option<Rc<Trie>> {
    Some(Rc::new(match &**t {
        Trie::Leaf(node) => Trie::Leaf(f(node)),
        Trie::Inner(children) => {
            let mut out: [Option<Rc<Trie>>; FAN] = Default::default();
            for (k, child) in children.iter().enumerate() {
                out[k] = child.as_ref().and_then(|c| map_trie(c, f));
            }
            Trie::Inner(out)
        }
    }))
}

fn same_trie(a: Option<&Rc<Trie>>, b: Option<&Rc<Trie>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            Rc::ptr_eq(x, y)
                || match (&**x, &**y) {
                    (Trie::Leaf(p), Trie::Leaf(q)) => same_node(p, q),
                    (Trie::Inner(p), Trie::Inner(q)) => p
                        .iter()
                        .zip(q)
                        .all(|(u, v)| same_trie(u.as_ref(), v.as_ref())),
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Деревья одной глубины: меньшее дорастает до большего.
fn aligned(a: &Moves, b: &Moves) -> (Moves, Moves) {
    let depth = a.depth.max(b.depth);
    let (mut a, mut b) = (a.clone(), b.clone());
    a.grow(depth);
    b.grow(depth);
    (a, b)
}

/// Слияние двух путей: перемещённое хоть на одном — перемещено.
fn merge(a: &State, b: &State) -> State {
    if !a.live {
        return b.clone();
    }
    if !b.live {
        return a.clone();
    }
    let (x, y) = aligned(&a.moved, &b.moved);
    let root = merge_trie(x.root.as_ref(), y.root.as_ref(), &merge_node, None);
    State {
        live: true,
        moved: Moves {
            depth: x.depth,
            root,
        },
    }
}

fn merge_node(a: &Node, b: &Node) -> Node {
    match (a, b) {
        (Node::Moved(m), _) | (_, Node::Moved(m)) => Node::Moved(*m),
        (Node::Full, Node::Full) => Node::Full,
        (Node::Parts(xs), Node::Full) | (Node::Full, Node::Parts(xs)) => Node::Parts(xs.clone()),
        (Node::Parts(xs), Node::Parts(ys)) => {
            Node::Parts(xs.iter().zip(ys).map(|(x, y)| merge_node(x, y)).collect())
        }
    }
}

fn merge_all(states: &[State]) -> State {
    states.iter().fold(State::dead(), |acc, s| merge(&acc, s))
}

/// Начало цикла `id`: вход, слитый с тем, что пришло по обратной дуге. Место, пустое только
/// из-за обратной дуги, перемещено в прошлом проходе этого цикла.
fn merge_head(entry: &State, back: &State, id: LoopId) -> State {
    if !back.live {
        return entry.clone();
    }
    if !entry.live {
        return back.clone();
    }
    let (x, y) = aligned(&entry.moved, &back.moved);
    let root = merge_trie(
        x.root.as_ref(),
        y.root.as_ref(),
        &|e, b| merge_head_node(e, b, id),
        Some(&|b| merge_head_node(&Node::Full, b, id)),
    );
    State {
        live: true,
        moved: Moves {
            depth: x.depth,
            root,
        },
    }
}

fn merge_head_node(entry: &Node, back: &Node, id: LoopId) -> Node {
    match (entry, back) {
        (Node::Moved(m), _) => Node::Moved(*m),
        (_, Node::Moved(m)) => Node::Moved(Moved {
            line: m.line,
            pass: Some(id),
        }),
        (Node::Full, Node::Full) => Node::Full,
        (Node::Parts(xs), Node::Full) => Node::Parts(xs.clone()),
        (Node::Full, Node::Parts(ys)) => Node::Parts(
            ys.iter()
                .map(|y| merge_head_node(&Node::Full, y, id))
                .collect(),
        ),
        (Node::Parts(xs), Node::Parts(ys)) => Node::Parts(
            xs.iter()
                .zip(ys)
                .map(|(x, y)| merge_head_node(x, y, id))
                .collect(),
        ),
    }
}

/// Состояния совпадают в том, что где держится; строки перемещений не сравниваются.
fn same(a: &State, b: &State) -> bool {
    if a.live != b.live {
        return false;
    }
    let (x, y) = aligned(&a.moved, &b.moved);
    same_trie(x.root.as_ref(), y.root.as_ref())
}

fn same_node(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Full, Node::Full) | (Node::Moved(_), Node::Moved(_)) => true,
        (Node::Parts(xs), Node::Parts(ys)) => xs.iter().zip(ys).all(|(x, y)| same_node(x, y)),
        _ => false,
    }
}

/// Первое перемещённое поле в записи, из которой перемещены поля, — путь к нему.
fn first_moved(node: &Node) -> Option<(Vec<usize>, Moved)> {
    match node {
        Node::Full => None,
        Node::Moved(m) => Some((Vec::new(), *m)),
        Node::Parts(fields) => fields.iter().enumerate().find_map(|(k, f)| {
            first_moved(f).map(|(mut path, m)| {
                path.insert(0, k);
                (path, m)
            })
        }),
    }
}

/// Место по пути `path` в `node` наполнено значением целиком. Возвращает, стало ли целым
/// всё `node`.
fn fill_node(node: &mut Node, path: &[Step]) -> bool {
    match path.first() {
        None => {
            *node = Node::Full;
            true
        }
        // массив держится только целиком
        Some(Step::Elem(_)) => matches!(node, Node::Full),
        Some(Step::Field(f)) => match node {
            Node::Full => true,
            // запись в часть перемещённого целиком значения отвергнута раньше
            Node::Moved(_) => false,
            Node::Parts(fields) => {
                if fill_node(&mut fields[*f], &path[1..])
                    && fields.iter().all(|x| matches!(x, Node::Full))
                {
                    *node = Node::Full;
                    true
                } else {
                    false
                }
            }
        },
    }
}

// ───────── обход ─────────

/// Куда идёт значение.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// В переменную, поле, кортеж, массив, вариант, параметр `take`, `return`, значение блока
    /// или операнд: непростое перемещается, простое копируется.
    Value,
    /// Параметру чтения или в `match` без слова: непростое читается на месте, простое
    /// копируется.
    Read,
    /// Параметру `mut` или в `match mut`: место меняется на месте.
    Mut,
}

/// Что делается с местом — для проверок, которые смотрят, не помешает ли это кому-то.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Копия или чтение на месте.
    Read,
    /// Запись в место.
    Write,
    /// Отдача как `mut`: параметру `mut` или в `match mut`.
    Lend,
    /// Перемещение.
    Take,
}

fn access(u: Use) -> Access {
    match u {
        Use::Copy | Use::Borrow => Access::Read,
        Use::BorrowMut => Access::Lend,
        Use::Move => Access::Take,
    }
}

/// Цикл, внутри которого идёт обход: что пришло к его `break` и `continue`.
struct LoopFrame {
    id: LoopId,
    /// Сколько областей имён открыто снаружи тела: имена из областей глубже за пределами
    /// прохода не видны.
    depth: usize,
    breaks: State,
    continues: State,
}

/// Вызов, чьи аргументы сейчас вычисляются: места, которые уже отданы ему для чтения или как
/// `mut` (решение 30).
struct CallFrame {
    func: FuncId,
    lent: Vec<(Place, Use)>,
}

/// Тело ветви с представлениями частей места (раздел 5, «Образцы»; решение 29).
struct Freeze {
    place: Place,
    /// `match mut`: к месту не обращаются вовсе; иначе его не меняют и не забирают.
    exclusive: bool,
}

/// Условие ветви: оно не меняет сравниваемое и привязки ветви (решение 15) и не забирает мест
/// (решение 31).
struct Guard {
    place: Option<Place>,
    bindings: Vec<LocalId>,
}

struct Walker<'a> {
    p: &'a Program,
    locals: &'a [Local],
    state: State,
    /// Имена, объявленные в каждом открытом блоке и ветви `match`. Когда область кончается, её
    /// имена уходят из состояния: дальше они не видны, а состояния остаются маленькими.
    scopes: Vec<Vec<LocalId>>,
    loops: Vec<LoopFrame>,
    /// Циклы, проход которых сейчас обходится, вместе с условием `while`.
    passes: Vec<LoopId>,
    /// Что пришло в начало цикла по обратной дуге в прошлый раз.
    back: HashMap<LoopId, State>,
    calls: Vec<CallFrame>,
    freezes: Vec<Freeze>,
    guards: Vec<Guard>,
    /// Идёт поиск состояния в начале цикла: нарушения не сообщаются, перемещение с нарушением
    /// не выполняется.
    quiet: bool,
}

impl<'a> Walker<'a> {
    fn new(p: &'a Program, locals: &'a [Local]) -> Walker<'a> {
        Walker {
            p,
            locals,
            state: State::live(),
            scopes: Vec::new(),
            loops: Vec::new(),
            passes: Vec::new(),
            back: HashMap::new(),
            calls: Vec::new(),
            freezes: Vec::new(),
            guards: Vec::new(),
            quiet: false,
        }
    }

    /// Место перемещено в прошлом проходе цикла, внутри которого идёт обход.
    fn earlier(&self, m: Moved) -> bool {
        m.pass.is_some_and(|id| self.passes.contains(&id))
    }

    /// Нарушение: ошибка, а пока ищется состояние цикла — ничего. Перемещение с нарушением
    /// вызвавший после этого не выполняет.
    fn violation(&self, line: u32, message: String) -> R<()> {
        if self.quiet {
            Ok(())
        } else {
            Err(CheckError { line, message })
        }
    }

    fn use_of(&self, ctx: Ctx, ty: &Type) -> Use {
        match (ctx, self.p.is_simple(ty)) {
            (Ctx::Mut, _) => Use::BorrowMut,
            (_, true) => Use::Copy,
            (Ctx::Value, false) => Use::Move,
            (Ctx::Read, false) => Use::Borrow,
        }
    }

    // ───────── блоки и инструкции ─────────

    fn block(&mut self, b: &mut Block) -> R<()> {
        self.scopes.push(Vec::new());
        for s in &mut b.stmts {
            self.stmt(s)?;
        }
        if let Some(v) = &mut b.value {
            self.expr(v, Ctx::Value)?;
        }
        self.close_scope();
        Ok(())
    }

    fn declare(&mut self, id: LocalId) {
        self.state.moved.remove(id);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(id);
        }
    }

    /// Область имён кончилась: её имена уходят из состояния.
    fn close_scope(&mut self) {
        for id in self.scopes.pop().unwrap_or_default() {
            self.state.moved.remove(id);
        }
    }

    /// Состояние, которое уходит по `break` или `continue` из областей глубже `depth`: имён из
    /// них после перехода не видно.
    fn leaving(&mut self, depth: usize) -> State {
        let mut state = std::mem::replace(&mut self.state, State::dead());
        for scope in self.scopes.iter().skip(depth) {
            for &id in scope {
                state.moved.remove(id);
            }
        }
        state
    }

    fn stmt(&mut self, s: &mut Stmt) -> R<()> {
        let line = s.line;
        match &mut s.kind {
            StmtKind::Let(id, e) => {
                self.expr(e, Ctx::Value)?;
                self.declare(*id);
            }
            StmtKind::LetTuple(pattern, e) => {
                self.expr(e, Ctx::Value)?;
                self.declare_tuple(pattern);
            }
            StmtKind::Assign(place, op, e) => self.assign(place, op.is_some(), e, line)?,
            StmtKind::Expr(e) => self.expr(e, Ctx::Value)?,
            StmtKind::While(id, cond, body) => self.repeat(*id, Some(cond), body)?,
            StmtKind::Loop(id, body) => self.repeat(*id, None, body)?,
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e, Ctx::Value)?;
                }
                self.state = State::dead();
            }
            StmtKind::Break(id) => {
                let Some(k) = self.loops.iter().rposition(|f| f.id == *id) else {
                    self.state = State::dead();
                    return Ok(());
                };
                let state = self.leaving(self.loops[k].depth);
                let f = &mut self.loops[k];
                f.breaks = merge(&f.breaks, &state);
            }
            StmtKind::Continue(id) => {
                let Some(k) = self.loops.iter().rposition(|f| f.id == *id) else {
                    self.state = State::dead();
                    return Ok(());
                };
                let state = self.leaving(self.loops[k].depth);
                let f = &mut self.loops[k];
                f.continues = merge(&f.continues, &state);
            }
            StmtKind::Assert(cond, _) => self.expr(cond, Ctx::Value)?,
        }
        Ok(())
    }

    fn declare_tuple(&mut self, pattern: &TuplePattern) {
        match pattern {
            TuplePattern::Bind(id) => self.declare(*id),
            TuplePattern::Skip => {}
            TuplePattern::Tuple(items) => {
                for item in items {
                    self.declare_tuple(item);
                }
            }
        }
    }

    /// `while` и `loop`. Сначала молча ищется состояние в начале прохода: тело проходится, пока
    /// оно не перестанет меняться. Затем тело проходится с этим состоянием и сообщением ошибок.
    /// Выход из `while` — по ложному условию и по `break`, из `loop` — только по `break`.
    fn repeat(&mut self, id: LoopId, mut cond: Option<&mut Expr>, body: &mut Block) -> R<()> {
        let entry = self.state.clone();
        let loud = !self.quiet;
        self.quiet = true;
        let mut head = match self.back.get(&id) {
            Some(back) => merge_head(&entry, back, id),
            None => entry.clone(),
        };
        let exit = loop {
            let (back, exit) = self.pass(id, cond.as_deref_mut(), body, &head)?;
            // новое начало вбирает прежнее: так оно только растёт и поиск кончается, хотя
            // пропущенное перемещение делает проход немонотонным
            let next = merge(&head, &merge_head(&entry, &back, id));
            self.back.insert(id, back);
            let done = same(&next, &head);
            head = next;
            if done {
                break exit;
            }
        };
        self.quiet = !loud;
        self.state = if loud {
            self.pass(id, cond, body, &head)?.1
        } else {
            exit
        };
        Ok(())
    }

    /// Один проход цикла от состояния `head`: что приходит обратно в начало и что выходит.
    fn pass(
        &mut self,
        id: LoopId,
        cond: Option<&mut Expr>,
        body: &mut Block,
        head: &State,
    ) -> R<(State, State)> {
        self.state = head.clone();
        self.passes.push(id);
        let r = self.pass_body(id, cond, body);
        self.passes.pop();
        r
    }

    fn pass_body(
        &mut self,
        id: LoopId,
        cond: Option<&mut Expr>,
        body: &mut Block,
    ) -> R<(State, State)> {
        let mut exit = State::dead();
        if let Some(cond) = cond {
            self.expr(cond, Ctx::Value)?;
            exit = self.state.clone();
        }
        self.loops.push(LoopFrame {
            id,
            depth: self.scopes.len(),
            breaks: State::dead(),
            continues: State::dead(),
        });
        let inner = self.block(body);
        let frame = self.loops.pop();
        inner?;
        let (breaks, continues) = match frame {
            Some(f) => (f.breaks, f.continues),
            None => (State::dead(), State::dead()),
        };
        let back = merge(&self.state, &continues);
        Ok((back, merge(&exit, &breaks)))
    }

    /// `место = значение` и составное присваивание: номера элементов, затем значение, затем
    /// запись (раздел 5, «Порядок»).
    fn assign(&mut self, place: &mut Place, compound: bool, e: &mut Expr, line: u32) -> R<()> {
        self.indices(place)?;
        self.expr(e, Ctx::Value)?;
        let problem = if compound {
            // составное присваивание читает простое значение и пишет его обратно
            self.availability(place)
        } else {
            self.write_problem(place)
        };
        let problem = problem.or_else(|| self.context_problem(place, Access::Write));
        // запись выполняется и с нарушением: иначе место осталось бы перемещённым, и в следующем
        // проходе цикла появилась бы ошибка, которой нет ни на одном пути
        if !compound {
            self.fill(place);
        }
        match problem {
            Some(message) => self.violation(line, message),
            None => Ok(()),
        }
    }

    // ───────── выражения ─────────

    fn expr(&mut self, e: &mut Expr, ctx: Ctx) -> R<()> {
        let line = e.line;
        let u = self.use_of(ctx, &e.ty);
        match &mut e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Char(_)
            | ExprKind::Unit => Ok(()),
            ExprKind::Place(place, mark) => {
                *mark = u;
                self.indices(place)?;
                self.use_place(place, u, line)
            }
            ExprKind::Part(base, steps, mark) => {
                *mark = u;
                self.expr(base, Ctx::Value)?;
                for step in steps.iter_mut() {
                    if let Step::Elem(i) = step {
                        self.expr(i, Ctx::Value)?;
                    }
                }
                if u == Use::Move && steps.iter().any(|s| matches!(s, Step::Elem(_))) {
                    return self.violation(
                        line,
                        "из временного массива не забирают: массив держится только целиком (E0508)"
                            .to_string(),
                    );
                }
                Ok(())
            }
            ExprKind::Unary(_, x) | ExprKind::Convert(_, x) => self.expr(x, Ctx::Value),
            ExprKind::Binary(first, rest) => {
                self.expr(first, Ctx::Value)?;
                for (_, x) in rest {
                    self.expr(x, Ctx::Value)?;
                }
                Ok(())
            }
            ExprKind::And(items) | ExprKind::Or(items) => {
                // следующая часть вычисляется не всегда: выходы после каждой части сливаются
                let mut outs = Vec::new();
                for x in items.iter_mut() {
                    self.expr(x, Ctx::Value)?;
                    outs.push(self.state.clone());
                }
                self.state = merge_all(&outs);
                Ok(())
            }
            ExprKind::Call(id, args) => self.call(*id, args),
            ExprKind::Record(_, fields) => {
                for (_, x) in fields {
                    self.expr(x, Ctx::Value)?;
                }
                Ok(())
            }
            ExprKind::Variant(_, _, value) => match value {
                VariantValue::None => Ok(()),
                VariantValue::One(x) => self.expr(x, Ctx::Value),
                VariantValue::Fields(fields) => {
                    for (_, x) in fields {
                        self.expr(x, Ctx::Value)?;
                    }
                    Ok(())
                }
            },
            ExprKind::Tuple(items) | ExprKind::Array(items) => {
                for x in items {
                    self.expr(x, Ctx::Value)?;
                }
                Ok(())
            }
            ExprKind::If(branches, otherwise) => {
                let mut outs = Vec::new();
                for (cond, block) in branches.iter_mut() {
                    self.expr(cond, Ctx::Value)?;
                    let after = self.state.clone();
                    self.block(block)?;
                    outs.push(std::mem::replace(&mut self.state, after));
                }
                if let Some(b) = otherwise {
                    self.block(b)?;
                }
                outs.push(self.state.clone());
                self.state = merge_all(&outs);
                Ok(())
            }
            ExprKind::Match(m) => self.match_expr(m),
        }
    }

    /// Номера элементов на пути к месту вычисляются раньше обращения к нему.
    fn indices(&mut self, place: &mut Place) -> R<()> {
        for step in place.path.iter_mut() {
            if let Step::Elem(i) = step {
                self.expr(i, Ctx::Value)?;
            }
        }
        Ok(())
    }

    /// Использование места; перемещение опустошает его.
    fn use_place(&mut self, place: &Place, u: Use, line: u32) -> R<()> {
        if let Some(message) = self.use_problem(place, u) {
            return self.violation(line, message);
        }
        if u == Use::Move {
            self.mark_moved(place, line);
        }
        Ok(())
    }

    /// Почему так обращаться к месту здесь нельзя: в нём нет значения целиком, его нельзя
    /// забрать, или это помешало бы условию ветви, представлениям или вызову.
    fn use_problem(&self, place: &Place, u: Use) -> Option<String> {
        self.availability(place)
            .or_else(|| {
                if u == Use::Move {
                    self.take_problem(place)
                } else {
                    None
                }
            })
            .or_else(|| self.context_problem(place, access(u)))
    }

    fn context_problem(&self, place: &Place, a: Access) -> Option<String> {
        self.guard_problem(place, a)
            .or_else(|| self.freeze_problem(place, a))
            .or_else(|| self.lent_problem(place, a))
    }

    /// В месте значение целиком: ни оно, ни путь к нему не перемещены, и из него ничего не
    /// забрано (E0382).
    fn availability(&self, place: &Place) -> Option<String> {
        let mut node = self.state.node(place.base);
        let mut k = 0;
        loop {
            match node {
                Node::Full => return None,
                Node::Moved(m) => {
                    let text = self.place_text(place.base, &place.path[..k]);
                    return Some(moved_message(&text, self.earlier(*m), m.line));
                }
                Node::Parts(fields) => match place.path.get(k) {
                    Some(Step::Field(f)) => {
                        node = &fields[*f];
                        k += 1;
                    }
                    _ => {
                        let (sub, m) = first_moved(node)?;
                        let whole = self.place_text(place.base, &place.path[..k]);
                        let mut steps: Vec<Step> = place.path[..k].to_vec();
                        steps.extend(sub.into_iter().map(Step::Field));
                        let part = self.place_text(place.base, &steps);
                        return Some(partial_message(&whole, &part, self.earlier(m), m.line));
                    }
                },
            }
        }
    }

    /// Почему это место нельзя забрать (раздел 5, таблица; решение 32 — у простого места
    /// сюда не доходят: оно копируется).
    fn take_problem(&self, place: &Place) -> Option<String> {
        let l = &self.locals[place.base];
        let what = if place.path.is_empty() {
            "его".to_string()
        } else {
            self.place_text(place.base, &place.path)
        };
        match l.kind {
            LocalKind::Param(Mode::Read) => {
                return Some(format!(
                    "{} — параметр для чтения: {} не забирают (E0507); забирает параметр take",
                    l.name, what
                ))
            }
            LocalKind::Param(Mode::Mut) => {
                return Some(format!(
                    "{} — параметр mut: {} не забирают, к концу функции он цел (раздел 5; E0507)",
                    l.name, what
                ))
            }
            LocalKind::Bound(Mode::Read) => {
                return Some(format!(
                    "{} — представление из match: {} не забирают, не возвращают и не кладут в переменную (12.62; E0507)",
                    l.name, what
                ))
            }
            LocalKind::Bound(Mode::Mut) => {
                return Some(format!(
                    "{} — представление из match mut: {} не забирают, не возвращают и не кладут в переменную (12.62; E0507)",
                    l.name, what
                ))
            }
            LocalKind::Let | LocalKind::Var | LocalKind::Param(Mode::Take) => {}
            LocalKind::Bound(Mode::Take) => {}
        }
        if place.path.iter().any(|s| matches!(s, Step::Elem(_))) {
            return Some(format!(
                "{}: из массива не забирают, массив держится только целиком (E0508); элемент читают на месте, заменяют записью или отдают как mut",
                self.place_text(place.base, &place.path)
            ));
        }
        None
    }

    /// Запись в часть — только если значение, которому она принадлежит, не перемещено целиком:
    /// частями наполняют то, из чего забраны части (раздел 5, «Использование»).
    fn write_problem(&self, place: &Place) -> Option<String> {
        let mut node = self.state.node(place.base);
        for (k, step) in place.path.iter().enumerate() {
            match node {
                Node::Full => return None,
                Node::Moved(m) => {
                    let text = self.place_text(place.base, &place.path[..k]);
                    return Some(whole_write_message(&text, self.earlier(*m), m.line));
                }
                Node::Parts(fields) => match step {
                    Step::Field(f) => node = &fields[*f],
                    Step::Elem(_) => return None,
                },
            }
        }
        None
    }

    /// Условие ветви ничего не забирает (решение 31) и не меняет сравниваемое, пересекающиеся с
    /// ним места и привязки ветви (решение 15).
    fn guard_problem(&self, place: &Place, a: Access) -> Option<String> {
        match a {
            Access::Read => None,
            Access::Take => self.guards.last().map(|_| {
                format!(
                    "условие ветви не забирает мест, а здесь забирается {} (решение 31)",
                    self.place_text(place.base, &place.path)
                )
            }),
            Access::Write | Access::Lend => self.guards.iter().rev().find_map(|g| {
                if g.bindings.contains(&place.base) {
                    let name = &self.locals[place.base].name;
                    return Some(match &g.place {
                        Some(x) => format!(
                            "{} — привязка этой ветви, часть сравниваемого {}: условие ветви её не меняет (решение 15; E0510)",
                            name,
                            self.place_text(x.base, &x.path)
                        ),
                        None => format!(
                            "{} — привязка этой ветви: условие ветви её не меняет (решение 15; E0510)",
                            name
                        ),
                    });
                }
                let x = g.place.as_ref().filter(|x| overlaps(place, x))?;
                Some(format!(
                    "условие ветви не меняет сравниваемое {} и пересекающиеся с ним места (решение 15; E0510)",
                    self.place_text(x.base, &x.path)
                ))
            }),
        }
    }

    /// Пока в теле ветви есть представления частей места, его и пересекающиеся с ним места не
    /// меняют и не забирают, а у `match mut` к ним не обращаются вовсе (раздел 5, решение 29).
    fn freeze_problem(&self, place: &Place, a: Access) -> Option<String> {
        self.freezes.iter().rev().find_map(|f| {
            if !overlaps(place, &f.place) {
                return None;
            }
            let x = self.place_text(f.place.base, &f.place.path);
            if f.exclusive {
                Some(format!(
                    "в ветви match mut {} с привязками к {} и пересекающимся с ним местам не обращаются: значение доступно через привязки (решение 29)",
                    x, x
                ))
            } else if a != Access::Read {
                Some(format!(
                    "пока в ветви есть представления частей {}, {} и пересекающиеся с ним места не меняют и не забирают (раздел 5)",
                    x, x
                ))
            } else {
                None
            }
        })
    }

    /// Место, уже отданное вызову для чтения или как `mut`, до начала вызова не меняют и не
    /// забирают — ни следующие аргументы, ни то, что в них (решение 30).
    fn lent_problem(&self, place: &Place, a: Access) -> Option<String> {
        if a == Access::Read {
            return None;
        }
        for frame in self.calls.iter().rev() {
            for (q, u) in &frame.lent {
                if !overlaps(place, q) {
                    continue;
                }
                let code = match (u, a) {
                    (_, Access::Take) => "E0505",
                    (_, Access::Write) => "E0506",
                    (Use::BorrowMut, _) => "E0499",
                    _ => "E0502",
                };
                return Some(self.lent_message(frame.func, q, *u, place, a, code));
            }
        }
        None
    }

    /// Непростое место, отданное аргументом для чтения, не пересекается с местом, отданным
    /// этому же вызову как `mut` (решение 30).
    fn aliased_read(&self, place: &Place) -> Option<String> {
        let frame = self.calls.last()?;
        frame
            .lent
            .iter()
            .find(|(q, u)| *u == Use::BorrowMut && overlaps(place, q))
            .map(|(q, u)| self.lent_message(frame.func, q, *u, place, Access::Read, "E0502"))
    }

    fn lent_message(
        &self,
        func: FuncId,
        q: &Place,
        u: Use,
        place: &Place,
        a: Access,
        code: &str,
    ) -> String {
        let how = if u == Use::BorrowMut {
            "как mut"
        } else {
            "для чтения"
        };
        let what = match a {
            Access::Read => "отдано для чтения",
            Access::Write => "записывается",
            Access::Lend => "отдано как mut",
            Access::Take => "забирается",
        };
        format!(
            "{} отдано вызову {} {}, а дальше в аргументах {} {}: места пересекаются (решение 30; {})",
            self.place_text(q.base, &q.path),
            self.p.funcs[func].name,
            how,
            self.place_text(place.base, &place.path),
            what,
            code
        )
    }

    /// Место перемещено. Перемещают только поля записей, без элементов массивов.
    fn mark_moved(&mut self, place: &Place, line: u32) {
        let moved = Node::Moved(Moved { line, pass: None });
        if place.path.iter().any(|s| matches!(s, Step::Elem(_))) {
            return;
        }
        let counts: Vec<usize> = self
            .path_types(place)
            .iter()
            .map(|ty| self.field_count(ty))
            .collect();
        let mut root = self.state.node(place.base).clone();
        let mut node = &mut root;
        for (step, &count) in place.path.iter().zip(&counts) {
            let Step::Field(f) = step else {
                return;
            };
            if let Node::Full = node {
                *node = Node::Parts(vec![Node::Full; count]);
            }
            match node {
                Node::Parts(fields) => node = &mut fields[*f],
                // путь уже перемещён: перемещать нечего
                _ => return,
            }
        }
        *node = moved;
        self.state.moved.set(place.base, root);
    }

    /// Место наполнено записью целиком; запись, где не осталось перемещённых полей, целая.
    fn fill(&mut self, place: &Place) {
        let mut node = self.state.node(place.base).clone();
        if matches!(node, Node::Full) {
            return;
        }
        fill_node(&mut node, &place.path);
        self.state.moved.set(place.base, node);
    }

    fn call(&mut self, id: FuncId, args: &mut [Arg]) -> R<()> {
        self.calls.push(CallFrame {
            func: id,
            lent: Vec::new(),
        });
        let r = self.args(args);
        self.calls.pop();
        r?;
        if self.p.is_never(&self.p.funcs[id].result) {
            self.state = State::dead();
        }
        Ok(())
    }

    /// Аргументы по порядку, каждый по режиму своего параметра. Отданное для чтения или как
    /// `mut` место запоминается: до вызова его не меняют и не забирают (решение 30).
    fn args(&mut self, args: &mut [Arg]) -> R<()> {
        for arg in args.iter_mut() {
            let ctx = match arg.mode {
                Mode::Read => Ctx::Read,
                Mode::Mut => Ctx::Mut,
                Mode::Take => Ctx::Value,
            };
            let line = arg.value.line;
            self.expr(&mut arg.value, ctx)?;
            let ExprKind::Place(place, u) = &arg.value.kind else {
                continue;
            };
            if !matches!(u, Use::Borrow | Use::BorrowMut) {
                continue;
            }
            if *u == Use::Borrow {
                if let Some(message) = self.aliased_read(place) {
                    self.violation(line, message)?;
                }
            }
            let lent = (place.clone(), *u);
            if let Some(frame) = self.calls.last_mut() {
                frame.lent.push(lent);
            }
        }
        Ok(())
    }

    // ───────── match ─────────

    /// `match`: значение, затем ветви — каждая с того же состояния. Условие ветви ничего не
    /// забирает, поэтому ветви ниже начинаются с того же, с чего ветвь с условием.
    fn match_expr(&mut self, m: &mut Match) -> R<()> {
        let line = m.value.line;
        // сравниваемое место; у временного значения его нет
        let place = match &m.value.kind {
            ExprKind::Place(place, _) => Some(place.clone()),
            _ => None,
        };
        // match take непростого места забирает его в начале каждого тела (решение 9)
        let takes = m.mode == Mode::Take && place.is_some() && !self.p.is_simple(&m.value.ty);
        let mut taken = false;
        if takes {
            if let ExprKind::Place(place, mark) = &mut m.value.kind {
                *mark = Use::Move;
                self.indices(place)?;
                match self.use_problem(place, Use::Move) {
                    Some(message) => self.violation(line, message)?,
                    None => taken = true,
                }
            }
        } else {
            let ctx = match m.mode {
                Mode::Read => Ctx::Read,
                Mode::Mut => Ctx::Mut,
                Mode::Take => Ctx::Value,
            };
            self.expr(&mut m.value, ctx)?;
        }
        let entry = self.state.clone();
        let mut outs = Vec::new();
        for case in &mut m.cases {
            self.state = entry.clone();
            let mut bindings = Vec::new();
            if let Some(first) = case.patterns.first() {
                bound(first, &mut bindings);
            }
            self.scopes.push(Vec::new());
            for &b in &bindings {
                self.declare(b);
            }
            if let Some(g) = &mut case.guard {
                self.guards.push(Guard {
                    place: place.clone(),
                    bindings: bindings.clone(),
                });
                let r = self.expr(g, Ctx::Value);
                self.guards.pop();
                r?;
            }
            if taken {
                if let Some(place) = &place {
                    self.mark_moved(place, line);
                }
            }
            // представления: у match mut — все привязки, у match без слова — непростые
            let views = match m.mode {
                Mode::Mut => !bindings.is_empty(),
                Mode::Read => bindings
                    .iter()
                    .any(|&b| !self.p.is_simple(&self.locals[b].ty)),
                Mode::Take => false,
            };
            let freeze = match (&place, views) {
                (Some(place), true) => Some(Freeze {
                    place: place.clone(),
                    exclusive: m.mode == Mode::Mut,
                }),
                _ => None,
            };
            let frozen = freeze.is_some();
            if let Some(f) = freeze {
                self.freezes.push(f);
            }
            let r = self.block(&mut case.body);
            if frozen {
                self.freezes.pop();
            }
            r?;
            self.close_scope();
            outs.push(std::mem::replace(&mut self.state, State::dead()));
        }
        if let Some(b) = &mut m.otherwise {
            self.state = entry.clone();
            if taken {
                if let Some(place) = &place {
                    self.mark_moved(place, line);
                }
            }
            self.block(b)?;
            outs.push(std::mem::replace(&mut self.state, State::dead()));
        }
        self.state = merge_all(&outs);
        Ok(())
    }

    // ───────── типы и текст ─────────

    /// Типы мест на пути: для каждого шага — тип того, из чего он идёт.
    fn path_types(&self, place: &Place) -> Vec<Type> {
        let mut ty = self.locals[place.base].ty.clone();
        let mut out = Vec::new();
        for step in &place.path {
            let next = self.step_type(&ty, step);
            out.push(ty);
            ty = next;
        }
        out
    }

    fn step_type(&self, ty: &Type, step: &Step) -> Type {
        match (ty, step) {
            (Type::Named(id), Step::Field(f)) => match &self.p.types[*id].kind {
                TypeKind::Struct(fields) => fields[*f].ty.clone(),
                TypeKind::Enum(_) => Type::Unit,
            },
            (Type::Array(item, _), Step::Elem(_)) => (**item).clone(),
            _ => Type::Unit,
        }
    }

    fn field_count(&self, ty: &Type) -> usize {
        match ty {
            Type::Named(id) => match &self.p.types[*id].kind {
                TypeKind::Struct(fields) => fields.len(),
                TypeKind::Enum(_) => 0,
            },
            _ => 0,
        }
    }

    /// Место текстом: `p.a.b`, `xs[i]`, `xs[2]`; номер сложнее имени — `xs[…]`.
    fn place_text(&self, base: LocalId, steps: &[Step]) -> String {
        let mut out = self.locals[base].name.clone();
        let mut ty = self.locals[base].ty.clone();
        for step in steps {
            match step {
                Step::Field(f) => {
                    if let Type::Named(id) = &ty {
                        if let TypeKind::Struct(fields) = &self.p.types[*id].kind {
                            out.push('.');
                            out.push_str(&fields[*f].name);
                        }
                    }
                }
                Step::Elem(i) => {
                    let index = match &i.kind {
                        ExprKind::Int(v) => v.to_string(),
                        ExprKind::Place(p, _) if p.path.is_empty() => {
                            self.locals[p.base].name.clone()
                        }
                        _ => "…".to_string(),
                    };
                    out.push('[');
                    out.push_str(&index);
                    out.push(']');
                }
            }
            ty = self.step_type(&ty, step);
        }
        out
    }
}

/// Имена, которые связывает образец.
fn bound(p: &Pattern, out: &mut Vec<LocalId>) {
    match &p.kind {
        PatternKind::Bind(id) => out.push(*id),
        PatternKind::Variant(_, _, items) | PatternKind::Tuple(items) => {
            for item in items {
                bound(item, out);
            }
        }
        PatternKind::Skip | PatternKind::Int(_) | PatternKind::Char(_) | PatternKind::Bool(_) => {}
    }
}

/// Места пересекаются, если путь одного продолжает путь другого. Элементы — если номера могут
/// совпасть: не совпадают только два разных числа (раздел 5, «Места»).
fn overlaps(a: &Place, b: &Place) -> bool {
    if a.base != b.base {
        return false;
    }
    for (x, y) in a.path.iter().zip(&b.path) {
        match (x, y) {
            (Step::Field(f), Step::Field(g)) if f != g => return false,
            (Step::Elem(i), Step::Elem(j)) => {
                if let (ExprKind::Int(u), ExprKind::Int(v)) = (&i.kind, &j.kind) {
                    if u != v {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    true
}

fn when(earlier: bool, line: u32) -> String {
    if earlier {
        format!("в прошлом проходе цикла, в строке {}", line)
    } else {
        format!("в строке {}", line)
    }
}

fn moved_message(place: &str, earlier: bool, line: u32) -> String {
    if earlier {
        format!(
            "{} перемещено {}: к следующему проходу его записывают заново (E0382)",
            place,
            when(earlier, line)
        )
    } else {
        format!(
            "{} перемещено {}: перемещённое не читают и не отдают, пока не запишут заново (E0382)",
            place,
            when(earlier, line)
        )
    }
}

fn partial_message(whole: &str, part: &str, earlier: bool, line: u32) -> String {
    format!(
        "из {} перемещено {} {}: целиком {} не используют, пока {} не записано заново (E0382)",
        whole,
        part,
        when(earlier, line),
        whole,
        part
    )
}

fn whole_write_message(place: &str, earlier: bool, line: u32) -> String {
    format!(
        "{} перемещено целиком {}: его наполняют только записью целиком (E0382)",
        place,
        when(earlier, line)
    )
}
