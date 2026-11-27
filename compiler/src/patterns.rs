//! Полнота `match` и недостижимые ветви (`spec/core-check.md`, раздел 6).
//!
//! Алгоритм — полезность образцов, как в rustc, где он взят из статьи Maranget «Warnings for
//! pattern matching». Альтернатива достижима, если есть значение, которое к ней подходит и не
//! подходит ни к одной альтернативе выше без условия. `match` полон, если каждое значение
//! подходит к какой-нибудь альтернативе без условия. К альтернативе, где у части типа нет
//! значений, не подходит ни одно значение, и она недостижима (решение 28).
//!
//! Все альтернативы проверяются за один обход значений, как в rustc. Значения делятся на части
//! по конструктору первого столбца — варианту, числу, символу, `bool` или кортежу, — строки
//! раскладываются по частям за один проход, и обход идёт в каждую часть. Строка достижима, если
//! в какой-то части значение доходит до неё раньше, чем до строки без условия выше. Часть, где
//! всё разобрала одна строка без условия, дальше не делится, а часть, которая ничего не добавит,
//! пропускается. В общем случае задача NP-полная, и на особых образцах обход бывает долгим.
//!
//! Обход идёт без рекурсии: строки — неизменяемые списки с общими хвостами, а отложенная работа
//! лежит в явном стеке. Поэтому стек потока не растёт ни с шириной, ни с глубиной образцов.
//! Рекурсия есть только по вложенности образца — в переводе, в причине недостижимости и в тексте
//! примера, — а её ограничивает предел вложенности разборщика.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::core::{Pattern, PatternKind, Payload, Program, Type, TypeId, TypeKind};
use crate::ir::Scalar;

/// Ветвь `match` для проверки: альтернативы по порядку и есть ли у ветви условие. У `else`
/// одна альтернатива — `None`: к ней подходит любое значение.
pub struct Row<'p> {
    pub alternatives: Vec<Option<&'p Pattern>>,
    pub guarded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Недостижима ветвь `row` — целиком, если `alternative` — `None`, иначе эта её
    /// альтернатива; номера — с нуля.
    Unreachable {
        row: usize,
        alternative: Option<usize>,
        cause: Cause,
    },
    /// Значение, которое не подходит ни к одной ветви без условия, — образцом, например
    /// `Op.Div` или `(_, false)`. `None`, если ветвей без условия нет, а образцы ветвей с
    /// условием — имена и `_`: тогда пример — любое значение.
    Missing(Option<String>),
}

/// Почему альтернатива недостижима.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// К ней не подходит ни одно значение: у этого типа в её образце значений нет (решение 28).
    NoValues(Type),
    /// Все её значения разобрала одна альтернатива выше без условия: номера её ветви и её самой,
    /// с нуля.
    CoveredBy { row: usize, alternative: usize },
    /// Её значения разобраны несколькими альтернативами выше вместе.
    Covered,
}

/// Альтернатива: где она, её образец (`None` — у `else`) и есть ли условие у её ветви.
struct Alt<'p> {
    row: usize,
    alternative: usize,
    pattern: Option<&'p Pattern>,
    guarded: bool,
}

/// Проверяет ветви `match` по значению типа `ty`: сначала недостижимое — первое по тексту,
/// затем полноту.
pub fn check(p: &Program, ty: &Type, rows: &[Row]) -> Result<(), Problem> {
    let mut alts = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (a, pattern) in row.alternatives.iter().enumerate() {
            alts.push(Alt {
                row: r,
                alternative: a,
                pattern: *pattern,
                guarded: row.guarded,
            });
        }
    }
    let (useful, witness) = search(p, ty, &alts);
    let mut start = 0;
    for (r, row) in rows.iter().enumerate() {
        let ids = start..start + row.alternatives.len();
        start = ids.end;
        let bad: Vec<usize> = ids.clone().filter(|&k| !useful[k]).collect();
        let Some(&first) = bad.first() else {
            continue;
        };
        // недостижимы все альтернативы — недостижима ветвь
        if bad.len() == row.alternatives.len() {
            return Err(Problem::Unreachable {
                row: r,
                alternative: None,
                cause: branch_cause(p, ty, &alts, ids.start, &bad),
            });
        }
        let cause = match no_values(p, ty, alts[first].pattern) {
            Some(t) => Cause::NoValues(t),
            None => covered_by(&alts, first, &[first]),
        };
        return Err(Problem::Unreachable {
            row: r,
            alternative: Some(alts[first].alternative),
            cause,
        });
    }
    match witness.as_ref().and_then(|w| w.head()) {
        None => Ok(()),
        Some(Pat::Wild) => Err(Problem::Missing(None)),
        Some(w) => Err(Problem::Missing(Some(show(p, w)))),
    }
}

/// Причина для ветви, все альтернативы которой недостижимы: у всех нет значений, или их все
/// разобрала одна альтернатива выше, или несколько вместе.
fn branch_cause(p: &Program, ty: &Type, alts: &[Alt], first: usize, ids: &[usize]) -> Cause {
    let mut with_values = Vec::new();
    let mut empty = None;
    for &k in ids {
        match no_values(p, ty, alts[k].pattern) {
            Some(t) => {
                empty.get_or_insert(t);
            }
            None => with_values.push(k),
        }
    }
    match empty {
        Some(t) if with_values.is_empty() => Cause::NoValues(t),
        _ => covered_by(alts, first, &with_values),
    }
}

/// Первая альтернатива до `before` без условия, которая одна разбирает все значения этих
/// альтернатив. Сравниваются только записи образцов — этого хватает, чтобы назвать ветвь в
/// сообщении.
fn covered_by(alts: &[Alt], before: usize, targets: &[usize]) -> Cause {
    for a in &alts[..before] {
        if !a.guarded && targets.iter().all(|&k| covers(a.pattern, alts[k].pattern)) {
            return Cause::CoveredBy {
                row: a.row,
                alternative: a.alternative,
            };
        }
    }
    Cause::Covered
}

/// Образец `p` подходит ко всем значениям образца `q`, и это видно по их записи. `None` — `else`.
fn covers(p: Option<&Pattern>, q: Option<&Pattern>) -> bool {
    let Some(p) = p else {
        return true;
    };
    if matches!(p.kind, PatternKind::Skip | PatternKind::Bind(_)) {
        return true;
    }
    let Some(q) = q else {
        return false;
    };
    let all =
        |ps: &[Pattern], qs: &[Pattern]| ps.iter().zip(qs).all(|(a, b)| covers(Some(a), Some(b)));
    match (&p.kind, &q.kind) {
        (PatternKind::Int(a), PatternKind::Int(b)) => a == b,
        (PatternKind::Char(a), PatternKind::Char(b)) => a == b,
        (PatternKind::Bool(a), PatternKind::Bool(b)) => a == b,
        (PatternKind::Variant(i, k, ps), PatternKind::Variant(j, l, qs)) => {
            i == j && k == l && all(ps, qs)
        }
        (PatternKind::Tuple(ps), PatternKind::Tuple(qs)) => all(ps, qs),
        _ => false,
    }
}

/// Тип без значений, из-за которого к образцу не подходит ни одно значение (решение 28), или
/// `None`, если значения к нему подходят. `None` в образце — `else` по значению типа `ty`.
fn no_values(p: &Program, ty: &Type, pattern: Option<&Pattern>) -> Option<Type> {
    let Some(pattern) = pattern else {
        return (!p.has_values(ty)).then(|| empty_part(p, ty));
    };
    match &pattern.kind {
        PatternKind::Skip | PatternKind::Bind(_) => {
            (!p.has_values(&pattern.ty)).then(|| empty_part(p, &pattern.ty))
        }
        PatternKind::Int(_) | PatternKind::Char(_) | PatternKind::Bool(_) => None,
        PatternKind::Variant(_, _, items) | PatternKind::Tuple(items) => items
            .iter()
            .find_map(|item| no_values(p, &item.ty, Some(item))),
    }
}

/// Тип без значений для сообщения: у кортежа — его часть без значений, у массива — элемент.
fn empty_part(p: &Program, ty: &Type) -> Type {
    let mut ty = ty;
    loop {
        match ty {
            Type::Tuple(items) => match items.iter().find(|item| !p.has_values(item)) {
                Some(item) => ty = item,
                None => break,
            },
            Type::Array(item, _) => ty = item,
            _ => break,
        }
    }
    ty.clone()
}

// ───────── образцы для обхода ─────────

/// Конструктор значения: то, по чему образцы различают значения.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Ctor {
    Bool(bool),
    Int(i128),
    Char(char),
    /// Вариант: перечисление и номер варианта.
    Variant(TypeId, usize),
    Tuple,
}

/// Образец для обхода: любое значение или конструктор с образцами частей. Имя — то же, что `_`.
#[derive(Debug, Clone)]
enum Pat {
    Wild,
    Ctor(Ctor, Rc<[Pat]>),
}

fn convert(pattern: &Pattern) -> Pat {
    let leaf = |c| Pat::Ctor(c, Rc::from(Vec::new()));
    match &pattern.kind {
        PatternKind::Skip | PatternKind::Bind(_) => Pat::Wild,
        PatternKind::Int(v) => leaf(Ctor::Int(*v)),
        PatternKind::Char(c) => leaf(Ctor::Char(*c)),
        PatternKind::Bool(b) => leaf(Ctor::Bool(*b)),
        PatternKind::Variant(id, k, items) => {
            Pat::Ctor(Ctor::Variant(*id, *k), items.iter().map(convert).collect())
        }
        PatternKind::Tuple(items) => Pat::Ctor(Ctor::Tuple, items.iter().map(convert).collect()),
    }
}

/// У значений с этим конструктором есть значения: у варианта — если они есть у всех его частей.
/// Остальные конструкторы встречаются только в частях, где значения есть.
fn ctor_has_values(p: &Program, c: &Ctor) -> bool {
    match c {
        Ctor::Variant(id, k) => variant_has_values(p, *id, *k),
        _ => true,
    }
}

fn variant_has_values(p: &Program, id: TypeId, k: usize) -> bool {
    match &p.types[id].kind {
        TypeKind::Enum(variants) => p.payload_has_values(&variants[k].payload),
        TypeKind::Struct(_) => false,
    }
}

// ───────── неизменяемый список ─────────

/// Значение списка, которое считается: у образцов — конструктор, а не `_`.
trait Counted: Clone {
    fn counted(&self) -> bool;
}

impl Counted for Pat {
    fn counted(&self) -> bool {
        matches!(self, Pat::Ctor(..))
    }
}

impl Counted for Type {
    fn counted(&self) -> bool {
        false
    }
}

/// Неизменяемый список с общими хвостами: образцы строки, типы её столбцов или пример.
struct List<T>(Option<Rc<Node<T>>>);

struct Node<T> {
    head: T,
    tail: List<T>,
    /// Сколько значений от этого узла до конца считается (`Counted`).
    counted: usize,
}

impl<T> Clone for List<T> {
    fn clone(&self) -> List<T> {
        List(self.0.clone())
    }
}

impl<T> Drop for List<T> {
    /// Длинный список удаляется циклом: рекурсия по хвостам переполнила бы стек.
    fn drop(&mut self) {
        let mut next = self.0.take();
        while let Some(node) = next {
            match Rc::try_unwrap(node) {
                Ok(mut node) => next = node.tail.0.take(),
                Err(_) => break,
            }
        }
    }
}

impl<T: Counted> List<T> {
    fn nil() -> List<T> {
        List(None)
    }

    fn cons(&self, head: T) -> List<T> {
        let counted = self.counted() + usize::from(head.counted());
        List(Some(Rc::new(Node {
            head,
            tail: self.clone(),
            counted,
        })))
    }

    fn head(&self) -> Option<&T> {
        self.0.as_ref().map(|n| &n.head)
    }

    fn tail(&self) -> List<T> {
        match &self.0 {
            Some(n) => n.tail.clone(),
            None => List(None),
        }
    }

    /// Сколько значений списка считается: у строки образцов `0` значит, что дальше одни `_`.
    fn counted(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.counted)
    }

    fn len(&self) -> usize {
        let mut n = 0;
        let mut rest = self.clone();
        while rest.head().is_some() {
            n += 1;
            rest = rest.tail();
        }
        n
    }

    /// Список с этими значениями впереди, по порядку.
    fn prepend(&self, items: &[T]) -> List<T> {
        let mut out = self.clone();
        for item in items.iter().rev() {
            out = out.cons(item.clone());
        }
        out
    }

    /// Первые `n` значений и остаток.
    fn split(&self, n: usize) -> (Vec<T>, List<T>) {
        let mut items = Vec::new();
        let mut rest = self.clone();
        for _ in 0..n {
            match rest.head() {
                Some(h) => items.push(h.clone()),
                None => break,
            }
            rest = rest.tail();
        }
        (items, rest)
    }
}

// ───────── обход ─────────

/// Строка обхода: альтернатива `id` и её образцы по столбцам.
struct Line {
    id: usize,
    pats: List<Pat>,
    /// Достижимость строки ещё решается здесь. Строка с `_` в столбце, где есть не все
    /// конструкторы типа, достижима в части с конструктором из столбца, только если достижима и
    /// в части без них, — поэтому в первой её не ищут (в rustc это «relevancy»).
    relevant: bool,
}

/// Часть значений, в которую идёт обход.
enum Child {
    /// Значения с этим конструктором; у него столько частей.
    Part(Ctor, usize),
    /// Значения с конструктором, которого в столбце нет.
    Rest,
}

/// Узел обхода, который делит значения по конструктору первого столбца.
struct Frame {
    ty: Type,
    tail: List<Type>,
    /// Части с конструктором, по порядку обхода, и строки с ним в первом столбце.
    parts: Vec<(Ctor, Vec<Line>)>,
    /// Строки с `_` в первом столбце: они идут во все части.
    wilds: Vec<Line>,
    /// Если в столбце есть не все конструкторы типа, после частей идёт часть без них, и пример
    /// из неё начинается с этого значения.
    missing: Option<Pat>,
    next: usize,
    rest_done: bool,
    /// Часть, которая сейчас обходится, и нужен ли из неё пример.
    current: Option<(Child, bool)>,
    /// Нужен ли из этого узла пример неразобранного значения.
    needed: bool,
    witness: Option<List<Pat>>,
}

impl Frame {
    /// Ответ части: пример неразобранного значения из неё или `None`.
    fn receive(&mut self, answer: Option<List<Pat>>) {
        let (Some(w), Some((child, true))) = (answer, &self.current) else {
            return;
        };
        if self.witness.is_some() {
            return;
        }
        self.witness = Some(match child {
            Child::Part(c, arity) => wrap(c.clone(), *arity, &w),
            Child::Rest => w.cons(self.missing.clone().unwrap_or(Pat::Wild)),
        });
    }
}

enum Entered {
    /// Узел решён сразу: пример неразобранного значения или `None`.
    Done(Option<List<Pat>>),
    Split(Box<Frame>),
}

struct Search<'p> {
    p: &'p Program,
    guarded: Vec<bool>,
    useful: Vec<bool>,
}

/// Достижимость альтернатив и пример неразобранного значения, если `match` не полон.
fn search(p: &Program, ty: &Type, alts: &[Alt]) -> (Vec<bool>, Option<List<Pat>>) {
    let mut s = Search {
        p,
        guarded: alts.iter().map(|a| a.guarded).collect(),
        useful: vec![false; alts.len()],
    };
    // у типа без значений разбирать нечего: ни одна альтернатива не достижима, а match полон
    if !p.has_values(ty) {
        return (s.useful, None);
    }
    let lines = alts
        .iter()
        .enumerate()
        .map(|(id, a)| Line {
            id,
            pats: List::nil().cons(a.pattern.map_or(Pat::Wild, convert)),
            relevant: true,
        })
        .collect();
    let witness = s.run(lines, List::nil().cons(ty.clone()));
    (s.useful, witness)
}

impl Search<'_> {
    fn run(&mut self, lines: Vec<Line>, cols: List<Type>) -> Option<List<Pat>> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut entered = self.enter(lines, cols, true);
        loop {
            let mut answer = match entered {
                Entered::Split(frame) => {
                    stack.push(*frame);
                    None
                }
                Entered::Done(w) => Some(w),
            };
            // ответ — узлу выше; затем следующая часть верхнего узла или его собственный ответ
            let (lines, cols, needed) = loop {
                let Some(top) = stack.last_mut() else {
                    return answer.flatten();
                };
                if let Some(w) = answer.take() {
                    top.receive(w);
                }
                match self.next_child(top) {
                    Some(child) => break child,
                    None => {
                        let done = stack.pop()?;
                        answer = Some(done.witness);
                    }
                }
            };
            entered = self.enter(lines, cols, needed);
        }
    }

    /// Узел со строками `lines` по столбцам типов `cols`. `needed` — нужен ли пример
    /// неразобранного значения.
    fn enter(&mut self, mut lines: Vec<Line>, cols: List<Type>, mut needed: bool) -> Entered {
        let Some(ty) = cols.head().cloned() else {
            // столбцов не осталось: значение доходит до строк по порядку, до первой без условия
            for l in &lines {
                self.useful[l.id] = true;
                if !self.guarded[l.id] {
                    return Entered::Done(None);
                }
            }
            return Entered::Done(Some(List::nil()));
        };
        // строка, где дальше одни `_`, подходит ко всем значениям части
        let mut skip = 0;
        for l in &lines {
            if l.pats.counted() > 0 {
                break;
            }
            self.useful[l.id] = true;
            if !self.guarded[l.id] {
                return Entered::Done(None);
            }
            skip += 1;
        }
        lines.drain(..skip);
        // ниже такой строки без условия ничего не достижимо, и неразобранных значений нет
        if let Some(j) = lines
            .iter()
            .position(|l| l.pats.counted() == 0 && !self.guarded[l.id])
        {
            lines.truncate(j + 1);
            needed = false;
        }
        if !needed && self.settled(&lines) {
            return Entered::Done(None);
        }
        if lines.is_empty() {
            return Entered::Done(Some(wilds(cols.len())));
        }
        let tail = cols.tail();
        let mut groups: BTreeMap<Ctor, Vec<Line>> = BTreeMap::new();
        let mut wild_lines = Vec::new();
        for l in lines {
            match l.pats.head().cloned() {
                Some(Pat::Ctor(c, _)) => {
                    // к варианту без значений не подходит ни одно значение (решение 28)
                    if ctor_has_values(self.p, &c) {
                        groups.entry(c).or_default().push(l);
                    }
                }
                _ => wild_lines.push(l),
            }
        }
        let sigma: BTreeSet<Ctor> = groups.keys().cloned().collect();
        let (order, missing) = match complete(self.p, &ty, &sigma) {
            Some(all) => (all, None),
            None => {
                let m = missing(self.p, &ty, &sigma);
                (sigma.into_iter().collect(), Some(m))
            }
        };
        let parts = order
            .into_iter()
            .map(|c| {
                let own = groups.remove(&c).unwrap_or_default();
                (c, own)
            })
            .collect();
        Entered::Split(Box::new(Frame {
            ty,
            tail,
            parts,
            wilds: wild_lines,
            missing,
            next: 0,
            rest_done: false,
            current: None,
            needed,
            witness: None,
        }))
    }

    /// Следующая часть узла: её строки, типы столбцов и нужен ли из неё пример. Часть, которая
    /// ничего не добавит, пропускается.
    fn next_child(&self, f: &mut Frame) -> Option<(Vec<Line>, List<Type>, bool)> {
        // строки с `_` решаются в части без конструкторов столбца, если она есть
        let wild_relevant = f.missing.is_none();
        while f.next < f.parts.len() {
            let k = f.next;
            f.next += 1;
            // в узле, где есть не все конструкторы, пример ищут только в части без них
            let needed = f.needed && f.witness.is_none() && f.missing.is_none();
            if !needed && self.settled(&f.parts[k].1) && (!wild_relevant || self.settled(&f.wilds))
            {
                continue;
            }
            let c = f.parts[k].0.clone();
            let own = std::mem::take(&mut f.parts[k].1);
            let parts = sub_types(self.p, &f.ty, &c);
            let wild_args = vec![Pat::Wild; parts.len()];
            let mut lines = Vec::with_capacity(own.len() + f.wilds.len());
            let mut own = own.into_iter().peekable();
            let mut wild = f.wilds.iter().peekable();
            // строки обеих групп — по порядку текста
            loop {
                let from_own = match (own.peek(), wild.peek()) {
                    (Some(a), Some(b)) => a.id < b.id,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => break,
                };
                if from_own {
                    if let Some(l) = own.next() {
                        let pats = match l.pats.head() {
                            Some(Pat::Ctor(_, args)) => l.pats.tail().prepend(args),
                            _ => l.pats.tail().prepend(&wild_args),
                        };
                        lines.push(Line {
                            id: l.id,
                            pats,
                            relevant: l.relevant,
                        });
                    }
                } else if let Some(l) = wild.next() {
                    lines.push(Line {
                        id: l.id,
                        pats: l.pats.tail().prepend(&wild_args),
                        relevant: l.relevant && wild_relevant,
                    });
                }
            }
            let cols = f.tail.prepend(&parts);
            f.current = Some((Child::Part(c, parts.len()), needed));
            return Some((lines, cols, needed));
        }
        if f.missing.is_some() && !f.rest_done {
            f.rest_done = true;
            let needed = f.needed && f.witness.is_none();
            if needed || !self.settled(&f.wilds) {
                let lines = f
                    .wilds
                    .iter()
                    .map(|l| Line {
                        id: l.id,
                        pats: l.pats.tail(),
                        relevant: l.relevant,
                    })
                    .collect();
                f.current = Some((Child::Rest, needed));
                return Some((lines, f.tail.clone(), needed));
            }
        }
        None
    }

    /// Обход ничего не добавит к этим строкам: достижимость каждой уже решена.
    fn settled(&self, lines: &[Line]) -> bool {
        lines.iter().all(|l| !l.relevant || self.useful[l.id])
    }
}

/// Пример из `n` значений `_`.
fn wilds(n: usize) -> List<Pat> {
    let mut out = List::nil();
    for _ in 0..n {
        out = out.cons(Pat::Wild);
    }
    out
}

fn wrap(c: Ctor, arity: usize, witness: &List<Pat>) -> List<Pat> {
    let (parts, rest) = witness.split(arity);
    rest.cons(Pat::Ctor(c, parts.into()))
}

/// Типы частей значения с конструктором `c`.
fn sub_types(p: &Program, ty: &Type, c: &Ctor) -> Vec<Type> {
    match (c, ty) {
        (Ctor::Tuple, Type::Tuple(parts)) => parts.clone(),
        (Ctor::Variant(id, k), _) => match &p.types[*id].kind {
            TypeKind::Enum(variants) => match &variants[*k].payload {
                Payload::None => Vec::new(),
                Payload::One(t) => vec![t.clone()],
                Payload::Fields(fields) => fields.iter().map(|f| f.ty.clone()).collect(),
            },
            TypeKind::Struct(_) => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Значений `char` — все скалярные значения Unicode, без суррогатов.
const CHAR_COUNT: u128 = 0x11_0000 - 0x800;

/// Если в столбце есть все конструкторы типа — их список для обхода, иначе `None`. У
/// перечисления конструкторы — варианты, у которых есть значения (решение 28).
fn complete(p: &Program, ty: &Type, sigma: &BTreeSet<Ctor>) -> Option<Vec<Ctor>> {
    match ty {
        Type::Scalar(Scalar::Bool) => {
            let all = [Ctor::Bool(false), Ctor::Bool(true)];
            all.iter().all(|c| sigma.contains(c)).then(|| all.to_vec())
        }
        Type::Scalar(Scalar::Char) => {
            (sigma.len() as u128 == CHAR_COUNT).then(|| sigma.iter().cloned().collect())
        }
        Type::Scalar(s) => match s.int_range() {
            // все значения целого перечислены по одному
            Some((lo, hi)) => ((sigma.len() as u128) == (hi - lo) as u128 + 1)
                .then(|| sigma.iter().cloned().collect()),
            None => None,
        },
        Type::Tuple(_) => sigma.contains(&Ctor::Tuple).then(|| vec![Ctor::Tuple]),
        Type::Named(id) => match &p.types[*id].kind {
            TypeKind::Enum(variants) => {
                let all: Vec<Ctor> = (0..variants.len())
                    .filter(|&k| variant_has_values(p, *id, k))
                    .map(|k| Ctor::Variant(*id, k))
                    .collect();
                all.iter().all(|c| sigma.contains(c)).then_some(all)
            }
            TypeKind::Struct(_) => None,
        },
        // у `Never` значений нет, и обход до него не доходит
        Type::Unit | Type::Array(..) | Type::Never => None,
    }
}

/// Значение для примера в столбце, где есть не все конструкторы: `_`, если нет ни одного,
/// иначе первый отсутствующий.
fn missing(p: &Program, ty: &Type, sigma: &BTreeSet<Ctor>) -> Pat {
    let leaf = |c| Pat::Ctor(c, Rc::from(Vec::new()));
    if sigma.is_empty() {
        return Pat::Wild;
    }
    match ty {
        Type::Scalar(Scalar::Bool) => {
            let b = sigma.contains(&Ctor::Bool(false));
            leaf(Ctor::Bool(b))
        }
        Type::Scalar(Scalar::Char) => leaf(Ctor::Char(missing_char(sigma))),
        Type::Scalar(s) => match s.int_range() {
            Some((lo, hi)) => leaf(Ctor::Int(missing_int(lo, hi, sigma))),
            None => Pat::Wild,
        },
        Type::Named(id) => match &p.types[*id].kind {
            TypeKind::Enum(variants) => {
                match (0..variants.len()).find(|&k| {
                    variant_has_values(p, *id, k) && !sigma.contains(&Ctor::Variant(*id, k))
                }) {
                    Some(k) => {
                        let c = Ctor::Variant(*id, k);
                        let arity = sub_types(p, ty, &c).len();
                        Pat::Ctor(c, vec![Pat::Wild; arity].into())
                    }
                    None => Pat::Wild,
                }
            }
            TypeKind::Struct(_) => Pat::Wild,
        },
        _ => Pat::Wild,
    }
}

/// Первое неразобранное целое: 0, 1, 2, … до наибольшего, затем −1, −2, …
fn missing_int(lo: i128, hi: i128, sigma: &BTreeSet<Ctor>) -> i128 {
    let mut v = lo.max(0);
    while v <= hi {
        if !sigma.contains(&Ctor::Int(v)) {
            return v;
        }
        v += 1;
    }
    let mut v = -1;
    while v >= lo {
        if !sigma.contains(&Ctor::Int(v)) {
            return v;
        }
        v -= 1;
    }
    lo
}

/// Первый неразобранный символ: сначала буквы и цифры, затем по порядку кодов.
fn missing_char(sigma: &BTreeSet<Ctor>) -> char {
    let free = |c: char| !sigma.contains(&Ctor::Char(c));
    ('a'..='z')
        .chain('0'..='9')
        .chain('A'..='Z')
        .chain((0..=0x10_FFFF).filter_map(char::from_u32))
        .find(|&c| free(c))
        .unwrap_or('a')
}

// ───────── текст примера ─────────

/// Пример — образцом Nulix: варианты с именем типа, поля по именам, `_` — любое значение.
fn show(p: &Program, pat: &Pat) -> String {
    let Pat::Ctor(c, args) = pat else {
        return "_".to_string();
    };
    match c {
        Ctor::Bool(b) => b.to_string(),
        Ctor::Int(v) => v.to_string(),
        Ctor::Char(ch) => char_text(*ch),
        Ctor::Tuple => {
            let parts: Vec<String> = args.iter().map(|a| show(p, a)).collect();
            format!("({})", parts.join(", "))
        }
        Ctor::Variant(id, k) => {
            let decl = &p.types[*id];
            let TypeKind::Enum(variants) = &decl.kind else {
                return "_".to_string();
            };
            let v = &variants[*k];
            let name = format!("{}.{}", decl.name, v.name);
            match &v.payload {
                Payload::None => name,
                Payload::One(_) => {
                    let inner = args.first().map_or_else(|| "_".to_string(), |a| show(p, a));
                    format!("{}({})", name, inner)
                }
                Payload::Fields(fields) => {
                    // поля до последнего разобранного — по именам, остальные — `..`
                    let Some(last) = args.iter().rposition(|a| !matches!(a, Pat::Wild)) else {
                        return format!("{}(..)", name);
                    };
                    let mut parts: Vec<String> = fields
                        .iter()
                        .zip(args.iter())
                        .take(last + 1)
                        .map(|(f, a)| match a {
                            Pat::Wild => "_".to_string(),
                            a => format!("{}: {}", f.name, show(p, a)),
                        })
                        .collect();
                    if last + 1 < fields.len() {
                        parts.push("..".to_string());
                    }
                    format!("{}({})", name, parts.join(", "))
                }
            }
        }
    }
}

/// Символ — литералом Nulix: видимые знаки как есть, остальные — `\u{…}` (13.16).
fn char_text(c: char) -> String {
    match c {
        '\'' => "'\\''".to_string(),
        '\\' => "'\\\\'".to_string(),
        c if c.is_ascii_graphic() || c == ' ' => format!("'{}'", c),
        c => format!("'\\u{{{:X}}}'", c as u32),
    }
}
