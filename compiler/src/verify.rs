//! Проверщик IR: «проверять, а не выводить».
//!
//! Проходит каждую функцию и каждый тест один раз сверху вниз. Тип и состояние владения каждой
//! переменной ведёт сам по инструкциям, а записанные состояния `[held: …]` сравнивает со своими
//! и ничего не выводит. Отвергает IR при первом расхождении. Правила — `spec/ir-core.md`,
//! раздел 3, подраздел «Проверка». Проверщик не зависит ни от проверки исходника, ни от
//! интерпретатора; принятый им IR интерпретатор исполняет без «неверного IR».

use crate::ir::*;
use crate::ir_text;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Почему IR отвергнут.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyError {
    /// Где: модуль, объявление или функция и путь к инструкции — номера инструкций в блоках
    /// с единицы и имена веток.
    pub at: String,
    pub message: String,
}

type R<T> = Result<T, VerifyError>;

/// Проверяет модуль целиком. Проверка рекурсивна по вложенности блоков и типов, поэтому
/// идёт на потоке с большим стеком (`stack::run_scoped`), как разбор и интерпретатор.
pub fn verify(m: &Module) -> R<()> {
    crate::stack::run_scoped(|| verify_module(m))
}

fn verify_module(m: &Module) -> R<()> {
    let d = Decls::new(m)?;
    for decl in &m.decls {
        match decl {
            Decl::Func(f) => {
                let mut b = Body::new(&d, format!("функция {}", f.name), f.result.clone());
                let mut st = St::new();
                for p in &f.params {
                    b.types.insert(p.name.clone(), p.ty.clone());
                    b.modes.insert(p.name.clone(), p.mode);
                    if p.mode == Mode::Take {
                        st.insert(p.name.clone(), Held::Full);
                    }
                }
                b.body(st, &f.body)?;
            }
            Decl::Test(t) => {
                let b = Body::new(&d, format!("тест «{}»", t.name), Type::Unit);
                b.body(St::new(), &t.body)?;
            }
            Decl::Struct(_) | Decl::Enum(_) => {}
        }
    }
    Ok(())
}

// ───────── объявления ─────────

struct Decls<'m> {
    structs: HashMap<&'m str, &'m StructDecl>,
    enums: HashMap<&'m str, &'m EnumDecl>,
    funcs: HashMap<&'m str, &'m Func>,
}

fn decl_err<T>(at: &str, message: String) -> R<T> {
    Err(VerifyError {
        at: at.to_string(),
        message,
    })
}

/// Имя записывается текстом IR: оно непусто, без обратной кавычки и управляющих символов.
fn name_ok(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c == '`' || c.is_control())
}

/// Имя функции: части через точку, каждая — имя без точки.
fn func_name_ok(s: &str) -> bool {
    s.split('.').all(name_ok)
}

/// Имя объявления; ссылки на имена проверяются тем, что они ведут к объявленному.
fn check_name(at: &str, what: &str, name: &str) -> R<()> {
    if name_ok(name) {
        Ok(())
    } else {
        decl_err(
            at,
            format!(
                "{} {:?}: имя нельзя записать текстом IR — оно пустое или с обратной кавычкой или управляющим символом",
                what, name
            ),
        )
    }
}

fn payload_types(p: &Payload) -> Vec<&Type> {
    match p {
        Payload::None => Vec::new(),
        Payload::One(t) => vec![t],
        Payload::Fields(fs) => fs.iter().map(|f| &f.ty).collect(),
    }
}

impl<'m> Decls<'m> {
    fn new(m: &'m Module) -> R<Self> {
        if m.version != VERSION {
            return decl_err(
                "модуль",
                format!("версия IR {} не поддерживается", m.version),
            );
        }
        let mut d = Decls {
            structs: HashMap::new(),
            enums: HashMap::new(),
            funcs: HashMap::new(),
        };
        for decl in &m.decls {
            match decl {
                Decl::Struct(s) => {
                    if d.is_type(&s.name) {
                        return decl_err("модуль", format!("тип {} объявлен дважды", s.name));
                    }
                    d.structs.insert(&s.name, s);
                }
                Decl::Enum(e) => {
                    if d.is_type(&e.name) {
                        return decl_err("модуль", format!("тип {} объявлен дважды", e.name));
                    }
                    d.enums.insert(&e.name, e);
                }
                Decl::Func(f) => {
                    if d.funcs.insert(&f.name, f).is_some() {
                        return decl_err("модуль", format!("функция {} объявлена дважды", f.name));
                    }
                }
                Decl::Test(_) => {}
            }
        }
        for decl in &m.decls {
            match decl {
                Decl::Struct(s) => d.check_struct(s)?,
                Decl::Enum(e) => d.check_enum(e)?,
                Decl::Func(f) => d.check_signature(f)?,
                Decl::Test(_) => {}
            }
        }
        d.no_recursion()?;
        Ok(d)
    }

    fn is_type(&self, name: &str) -> bool {
        self.structs.contains_key(name) || self.enums.contains_key(name)
    }

    fn check_type(&self, t: &Type) -> Result<(), String> {
        match t {
            Type::Scalar(_) | Type::Unit => Ok(()),
            Type::Tuple(ts) if ts.len() < 2 => Err("в кортеже меньше двух элементов".to_string()),
            Type::Tuple(ts) => ts.iter().try_for_each(|t| self.check_type(t)),
            Type::Array(t, _) => self.check_type(t),
            Type::Named(n) if self.is_type(n) => Ok(()),
            Type::Named(n) => Err(format!("нет типа {}", n)),
        }
    }

    /// Имена полей не повторяются, типы полей объявлены.
    fn check_fields(&self, at: &str, fields: &[Field]) -> R<()> {
        let mut seen = HashSet::new();
        for f in fields {
            check_name(at, "поле", &f.name)?;
            if !seen.insert(f.name.as_str()) {
                return decl_err(at, format!("поле {} повторено", f.name));
            }
            if let Err(e) = self.check_type(&f.ty) {
                return decl_err(at, format!("поле {}: {}", f.name, e));
            }
        }
        Ok(())
    }

    fn check_struct(&self, s: &StructDecl) -> R<()> {
        let at = format!("запись {}", s.name);
        check_name(&at, "запись", &s.name)?;
        self.check_fields(&at, &s.fields)?;
        if s.copy {
            if let Some(f) = s.fields.iter().find(|f| !self.is_copy(&f.ty)) {
                return decl_err(&at, format!("простая запись с непростым полем {}", f.name));
            }
        }
        Ok(())
    }

    fn check_enum(&self, e: &EnumDecl) -> R<()> {
        let at = format!("перечисление {}", e.name);
        check_name(&at, "перечисление", &e.name)?;
        let mut seen = HashSet::new();
        for v in &e.variants {
            check_name(&at, "вариант", &v.name)?;
            if !seen.insert(v.name.as_str()) {
                return decl_err(&at, format!("вариант {} повторён", v.name));
            }
            match &v.payload {
                Payload::None => {}
                Payload::One(t) => {
                    if let Err(e) = self.check_type(t) {
                        return decl_err(&at, format!("вариант {}: {}", v.name, e));
                    }
                }
                Payload::Fields(fs) if fs.is_empty() => {
                    return decl_err(&at, format!("у варианта {} пустой список полей", v.name))
                }
                Payload::Fields(fs) => {
                    self.check_fields(&format!("{}, вариант {}", at, v.name), fs)?
                }
            }
            if e.copy && !payload_types(&v.payload).iter().all(|t| self.is_copy(t)) {
                return decl_err(
                    &at,
                    format!(
                        "простое перечисление с непростым значением в варианте {}",
                        v.name
                    ),
                );
            }
        }
        Ok(())
    }

    fn check_signature(&self, f: &Func) -> R<()> {
        let at = format!("функция {}", f.name);
        if !func_name_ok(&f.name) {
            return decl_err(
                &at,
                format!(
                    "функция {:?}: имя нельзя записать текстом IR — каждая часть между точками должна быть непустой, без обратной кавычки и управляющих символов",
                    f.name
                ),
            );
        }
        if !f.uses.is_empty() {
            return decl_err(&at, "требований uses на шаге 2 нет".to_string());
        }
        let mut seen = HashSet::new();
        for p in &f.params {
            check_name(&at, "параметр", &p.name)?;
            if !seen.insert(p.name.as_str()) {
                return decl_err(&at, format!("параметр {} повторён", p.name));
            }
            if let Err(e) = self.check_type(&p.ty) {
                return decl_err(&at, format!("параметр {}: {}", p.name, e));
            }
        }
        if let Err(e) = self.check_type(&f.result) {
            return decl_err(&at, format!("результат: {}", e));
        }
        Ok(())
    }

    /// Имена типов, которые тип содержит прямо, без косвенности.
    fn contained<'t>(t: &'t Type, out: &mut Vec<&'t str>) {
        match t {
            Type::Named(n) => out.push(n),
            Type::Tuple(ts) => ts.iter().for_each(|t| Self::contained(t, out)),
            Type::Array(t, _) => Self::contained(t, out),
            Type::Scalar(_) | Type::Unit => {}
        }
    }

    fn parts(&self, name: &str) -> Vec<&'m str> {
        let mut out = Vec::new();
        if let Some(s) = self.structs.get(name) {
            s.fields
                .iter()
                .for_each(|f| Self::contained(&f.ty, &mut out));
        } else if let Some(e) = self.enums.get(name) {
            for v in &e.variants {
                payload_types(&v.payload)
                    .into_iter()
                    .for_each(|t| Self::contained(t, &mut out));
            }
        }
        out
    }

    /// Тип не содержит сам себя без косвенности: кучи на шаге 2 нет.
    fn no_recursion(&self) -> R<()> {
        // 1 — тип в обходе, 2 — проверен
        let mut mark: HashMap<&str, u8> = HashMap::new();
        let mut names: Vec<&str> = self
            .structs
            .keys()
            .chain(self.enums.keys())
            .copied()
            .collect();
        names.sort();
        for start in names {
            if mark.contains_key(start) {
                continue;
            }
            mark.insert(start, 1);
            let mut stack: Vec<(&str, Vec<&str>)> = vec![(start, self.parts(start))];
            while let Some((name, rest)) = stack.last_mut() {
                let name: &str = name;
                match rest.pop() {
                    Some(next) => match mark.get(next) {
                        Some(1) => {
                            return decl_err(
                                &format!("тип {}", next),
                                format!("тип {} содержит сам себя без косвенности", next),
                            )
                        }
                        Some(_) => {}
                        None => {
                            mark.insert(next, 1);
                            let parts = self.parts(next);
                            stack.push((next, parts));
                        }
                    },
                    None => {
                        mark.insert(name, 2);
                        stack.pop();
                    }
                }
            }
        }
        Ok(())
    }

    fn is_copy(&self, t: &Type) -> bool {
        match t {
            Type::Scalar(_) | Type::Unit => true,
            Type::Tuple(ts) => ts.iter().all(|t| self.is_copy(t)),
            Type::Array(t, _) => self.is_copy(t),
            Type::Named(n) => match (self.structs.get(n.as_str()), self.enums.get(n.as_str())) {
                (Some(s), _) => s.copy,
                (_, Some(e)) => e.copy,
                _ => false,
            },
        }
    }

    /// Тип без значений: перечисление без вариантов.
    fn is_never(&self, t: &Type) -> bool {
        match t {
            Type::Named(n) => self
                .enums
                .get(n.as_str())
                .is_some_and(|e| e.variants.is_empty()),
            _ => false,
        }
    }

    /// Части записи или кортежа: тип и то, как часть дописывается к месту.
    fn child_types(&self, t: &Type) -> Vec<(Type, String)> {
        match t {
            Type::Tuple(ts) => ts
                .iter()
                .enumerate()
                .map(|(k, t)| (t.clone(), format!(".{}", k)))
                .collect(),
            Type::Named(n) => match self.structs.get(n.as_str()) {
                Some(s) => s
                    .fields
                    .iter()
                    .map(|f| (f.ty.clone(), format!(".{}", f.name)))
                    .collect(),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Значения варианта: тип и то, как значение дописывается к месту.
    fn variant_types(&self, t: &Type, vi: usize) -> Vec<(Type, String)> {
        let var = match t {
            Type::Named(n) => self.enums.get(n.as_str()).and_then(|e| e.variants.get(vi)),
            _ => None,
        };
        match var {
            Some(v) => match &v.payload {
                Payload::None => Vec::new(),
                Payload::One(t) => vec![(t.clone(), format!(".{}.0", v.name))],
                Payload::Fields(fs) => fs
                    .iter()
                    .map(|f| (f.ty.clone(), format!(".{}.{}", v.name, f.name)))
                    .collect(),
            },
            None => Vec::new(),
        }
    }
}

// ───────── места ─────────

/// Может ли место `a` содержать место `b`, то есть быть его началом. Элементы массива, номера
/// которых могут совпасть, считаются одним местом: не совпадают только два разных числа.
fn may_contain(a: &Place, b: &Place) -> bool {
    a.base == b.base
        && a.path.len() <= b.path.len()
        && a.path.iter().zip(&b.path).all(|pair| match pair {
            (Seg::Elem(Index::Lit(m)), Seg::Elem(Index::Lit(n))) => m == n,
            (Seg::Elem(_), Seg::Elem(_)) => true,
            (x, y) => x == y,
        })
}

/// Пересекаются ли места: одно может содержать другое.
fn overlaps(a: &Place, b: &Place) -> bool {
    may_contain(a, b) || may_contain(b, a)
}

/// Переменные, которые дают номера элементов на пути к месту.
fn index_vars(p: &Place) -> impl Iterator<Item = &str> {
    p.path.iter().filter_map(|s| match s {
        Seg::Elem(Index::Var(v)) => Some(v.as_str()),
        _ => None,
    })
}

/// Места, которые блок может переписать: записи, определения и места, отданные вызову для
/// изменения. Опустошение — `move`, `drop`, `local` — вариант не меняет: пустое место не
/// читается, а наполнить его снова можно только записью или определением.
fn written(b: &Block, out: &mut Vec<Place>) {
    fn lent(r: &Rhs, out: &mut Vec<Place>) {
        if let Rhs::Call { args, .. } = r {
            lent_args(args, out);
        }
    }
    fn lent_args(args: &[Operand], out: &mut Vec<Place>) {
        for o in args {
            if let Operand::BorrowMut(p) = o {
                out.push(p.clone());
            }
        }
    }
    for s in &b.stmts {
        match s {
            Stmt::Define { name, value, .. } => {
                lent(value, out);
                out.push(Place {
                    base: name.clone(),
                    path: Vec::new(),
                });
            }
            Stmt::Assign { place, value } => {
                lent(value, out);
                out.push(place.clone());
            }
            Stmt::Call { args, .. } => lent_args(args, out),
            Stmt::If {
                then, otherwise, ..
            } => {
                written(then, out);
                written(otherwise, out);
            }
            Stmt::Switch {
                cases, otherwise, ..
            } => {
                cases.iter().for_each(|c| written(&c.body, out));
                if let Some(b) = otherwise {
                    written(b, out);
                }
            }
            Stmt::Loop { body, .. } => written(body, out),
            Stmt::Local { .. }
            | Stmt::Drop(_)
            | Stmt::Return(_)
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Crash(_) => {}
        }
    }
}

// ───────── состояние владения ─────────

/// Что держит место: ничего, всё значение, части записи или кортежа, значения известного
/// варианта. Части, из которых всё перемещено, остаются частями: значение без них — не то же,
/// что пустое место, его можно уничтожить или дописать.
#[derive(Debug, Clone, PartialEq)]
enum Held {
    Empty,
    Full,
    Parts(Vec<Held>),
    Variant(usize, Vec<Held>),
}

/// Что держат переменные функции. Здесь только переменные, которыми функция владеет:
/// локальные и забранные параметры. Параметры чтения и `mut` всегда целы.
type St = BTreeMap<String, Held>;

/// Части, все целые, — это целое значение.
fn normalize(h: &mut Held) {
    if let Held::Parts(cs) | Held::Variant(_, cs) = h {
        if !cs.is_empty() && cs.iter().all(|c| *c == Held::Full) {
            *h = Held::Full;
        }
    }
}

/// Шаг пути к месту, сверенный с типами.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Field(usize),
    Pos(usize),
    /// Значение варианта: номер варианта и номер значения.
    Variant(usize, usize),
    /// Элемент массива: массив держится только целиком.
    Elem,
}

fn get_in(h: &Held, steps: &[Step]) -> Held {
    let mut cur = h;
    for s in steps {
        let next = match (cur, s) {
            (Held::Full, _) => return Held::Full,
            (Held::Empty, _) => return Held::Empty,
            (Held::Parts(cs), Step::Field(k) | Step::Pos(k)) => cs.get(*k),
            (Held::Variant(vi, cs), Step::Variant(vj, k)) if vi == vj => cs.get(*k),
            _ => None,
        };
        cur = match next {
            Some(c) => c,
            None => return Held::Empty,
        };
    }
    cur.clone()
}

/// Какие операции есть у какого скаляра (решения 19 и 20 ноября 2026).
fn op_allowed(op: Op, ty: Scalar) -> bool {
    let int = ty.int_range().is_some();
    let float = matches!(ty, Scalar::F32 | Scalar::F64);
    match op {
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => int || float,
        Op::Neg => ty.is_signed() || float,
        Op::And | Op::Or | Op::Xor | Op::Not => int || ty == Scalar::Bool,
        Op::Shl | Op::Shr => int,
        Op::Eq | Op::Ne => true,
        Op::Lt | Op::Le | Op::Gt | Op::Ge => int || float || ty == Scalar::Char,
    }
}

/// `bool` ни во что не преобразуется; символ и дробное друг в друга — тоже.
fn conv_allowed(from: Scalar, to: Scalar) -> bool {
    let kind = |s: Scalar| match s {
        Scalar::Bool => 0,
        Scalar::Char => 1,
        Scalar::F32 | Scalar::F64 => 2,
        _ => 3,
    };
    let (a, b) = (kind(from), kind(to));
    a != 0 && b != 0 && !matches!((a, b), (1, 2) | (2, 1))
}

// ───────── тело функции ─────────

struct LoopFrame {
    label: String,
    head: St,
    /// Состояние после цикла: как записано и разобранное — при первом выходе.
    after_text: Option<State>,
    after: Option<St>,
}

/// Известный вариант места: внутри ветки `case`, пока место не могло быть переписано.
struct Fact {
    place: Place,
    variant: usize,
    live: bool,
}

struct Body<'d, 'm> {
    d: &'d Decls<'m>,
    what: String,
    result: Type,
    /// Типы параметров и переменных, объявленных выше по тексту.
    types: HashMap<String, Type>,
    modes: HashMap<String, Mode>,
    loops: Vec<LoopFrame>,
    facts: Vec<Fact>,
    /// Путь к текущей инструкции — для сообщений.
    path: Vec<String>,
}

impl<'d, 'm> Body<'d, 'm> {
    fn new(d: &'d Decls<'m>, what: String, result: Type) -> Self {
        Body {
            d,
            what,
            result,
            types: HashMap::new(),
            modes: HashMap::new(),
            loops: Vec::new(),
            facts: Vec::new(),
            path: Vec::new(),
        }
    }

    fn fail<T>(&self, message: String) -> R<T> {
        let mut at = self.what.clone();
        for p in &self.path {
            at.push_str(" › ");
            at.push_str(p);
        }
        Err(VerifyError { at, message })
    }

    /// Тело целиком: если конец достижим, это возврат `()`.
    fn body(mut self, mut st: St, b: &Block) -> R<()> {
        if self.block(&mut st, b)? {
            if self.result != Type::Unit {
                return self.fail(format!(
                    "конец тела достижим, а результат {}",
                    ir_text::ty(&self.result)
                ));
            }
            self.no_owned(&st)?;
        }
        Ok(())
    }

    fn ty_of(&self, name: &str) -> Type {
        self.types.get(name).cloned().unwrap_or(Type::Unit)
    }

    fn func(&self, name: &str) -> R<&'m Func> {
        match self.d.funcs.get(name) {
            Some(f) => Ok(*f),
            None => self.fail(format!("нет функции {}", name)),
        }
    }

    fn check_type(&self, t: &Type) -> R<()> {
        match self.d.check_type(t) {
            Ok(()) => Ok(()),
            Err(e) => self.fail(e),
        }
    }

    fn same_type(&self, got: &Type, expected: &Type) -> R<()> {
        if got == expected {
            Ok(())
        } else {
            self.fail(format!(
                "значение типа {} там, где ждут {}",
                ir_text::ty(got),
                ir_text::ty(expected)
            ))
        }
    }

    // ── владение ──

    fn get(&self, st: &St, base: &str, steps: &[Step]) -> Held {
        match self.modes.get(base) {
            Some(Mode::Read) | Some(Mode::Mut) => Held::Full,
            _ => get_in(st.get(base).unwrap_or(&Held::Empty), steps),
        }
    }

    /// Первое место внутри, где держится непростое значение.
    fn first_owned(&self, h: &Held, ty: &Type, at: String) -> Option<String> {
        let inner = |cs: &[Held], ts: Vec<(Type, String)>| {
            cs.iter()
                .zip(ts)
                .find_map(|(c, (t, s))| self.first_owned(c, &t, format!("{}{}", at, s)))
        };
        match h {
            Held::Empty => None,
            Held::Full if self.d.is_copy(ty) => None,
            Held::Full => Some(at),
            Held::Parts(cs) => inner(cs, self.d.child_types(ty)),
            Held::Variant(vi, cs) => inner(cs, self.d.variant_types(ty, *vi)),
        }
    }

    /// Меняет то, что держит место по пути; целое или пустое расщепляется на части.
    fn set_at(&self, h: &mut Held, ty: &Type, steps: &[Step], new: Held) -> R<()> {
        let (first, rest) = match steps.split_first() {
            Some(x) => x,
            None => {
                *h = new;
                return Ok(());
            }
        };
        match *first {
            Step::Field(k) | Step::Pos(k) => {
                let ts = self.d.child_types(ty);
                if matches!(h, Held::Full | Held::Empty) {
                    *h = Held::Parts(vec![h.clone(); ts.len()]);
                }
                match (h.borrow_parts(), ts.get(k)) {
                    (Some(cs), Some((t, _))) if k < cs.len() => self.set_at(&mut cs[k], t, rest, new)?,
                    _ => return self.fail("путь не подходит к значению".to_string()),
                }
            }
            Step::Variant(vi, k) => {
                let ts = self.d.variant_types(ty, vi);
                if matches!(h, Held::Full | Held::Empty) {
                    *h = Held::Variant(vi, vec![h.clone(); ts.len()]);
                }
                match (&mut *h, ts.get(k)) {
                    (Held::Variant(vj, cs), Some((t, _))) if *vj == vi && k < cs.len() => {
                        self.set_at(&mut cs[k], t, rest, new)?
                    }
                    _ => return self.fail("в месте другой вариант".to_string()),
                }
            }
            Step::Elem => {
                return self.fail(
                    "элемент массива отдельно не забирают и не уничтожают: массив держится только целиком"
                        .to_string(),
                )
            }
        }
        normalize(h);
        Ok(())
    }

    fn set(&self, st: &mut St, base: &str, steps: &[Step], new: Held) -> R<()> {
        let ty = self.ty_of(base);
        let h = st.entry(base.to_string()).or_insert(Held::Empty);
        self.set_at(h, &ty, steps, new)?;
        if *h == Held::Empty {
            st.remove(base);
        }
        Ok(())
    }

    /// Место принадлежит функции: локальная переменная или забранный параметр.
    fn owned(&self, p: &Place, what: &str) -> R<()> {
        match self.modes.get(&p.base) {
            Some(Mode::Read) | Some(Mode::Mut) => self.fail(format!(
                "{} {}: параметр {} функции не принадлежит",
                what,
                ir_text::place(p),
                p.base
            )),
            _ => Ok(()),
        }
    }

    fn no_owned(&self, st: &St) -> R<()> {
        for (name, h) in st {
            if let Some(at) = self.first_owned(h, &self.ty_of(name), name.clone()) {
                return self.fail(format!("значение в {} не отдано и не уничтожено", at));
            }
        }
        Ok(())
    }

    /// Сходится ли то, что держит место здесь, с записанным: записанное держится, а сверх
    /// него — только простое, которое забывается.
    fn fits(&self, s: &Held, r: &Held, ty: &Type, at: &str) -> Result<(), String> {
        match r {
            Held::Empty => match self.first_owned(s, ty, at.to_string()) {
                None => Ok(()),
                Some(inner) => Err(format!(
                    "в {} непростое значение, а записанное состояние его не называет",
                    inner
                )),
            },
            Held::Full if *s == Held::Full => Ok(()),
            Held::Full if *s == Held::Empty => Err(format!(
                "записанное состояние называет {}, а значения там нет",
                at
            )),
            Held::Full => Err(format!(
                "записанное состояние называет {} целиком, а часть значения перемещена",
                at
            )),
            Held::Parts(rs) => {
                let ts = self.d.child_types(ty);
                for (k, rk) in rs.iter().enumerate() {
                    let sk = match s {
                        Held::Full => Held::Full,
                        Held::Empty => Held::Empty,
                        Held::Parts(ss) => ss.get(k).cloned().unwrap_or(Held::Empty),
                        Held::Variant(..) => return Err(format!("{}: значение не того вида", at)),
                    };
                    match ts.get(k) {
                        Some((t, suffix)) => self.fits(&sk, rk, t, &format!("{}{}", at, suffix))?,
                        None => return Err(format!("{}: значение не того вида", at)),
                    }
                }
                Ok(())
            }
            Held::Variant(..) => {
                Err("записанное состояние не называет значений вариантов".to_string())
            }
        }
    }

    /// Сверка своего состояния с записанным.
    fn fits_state(&self, s: &St, r: &St, context: &str) -> R<()> {
        let names: BTreeSet<&String> = s.keys().chain(r.keys()).collect();
        for name in names {
            let sh = s.get(name).unwrap_or(&Held::Empty);
            let rh = r.get(name).unwrap_or(&Held::Empty);
            if let Err(e) = self.fits(sh, rh, &self.ty_of(name), name) {
                return self.fail(format!("{}: {}", context, e));
            }
        }
        Ok(())
    }

    /// Записанное состояние: переменные, которыми функция владеет, их поля и элементы
    /// кортежей — без повторов и пересечений; место, у которого названы все части, пишется
    /// целиком.
    fn record(&self, rec: &State) -> R<St> {
        let mut st = St::new();
        let mut listed: Vec<(&Place, Vec<Step>)> = Vec::new();
        for p in &rec.held {
            let text = ir_text::place(p);
            if matches!(self.modes.get(&p.base), Some(Mode::Read) | Some(Mode::Mut)) {
                return self.fail(format!(
                    "в записанном состоянии {}: параметры чтения и mut не называются",
                    text
                ));
            }
            let mut ty = match self.types.get(&p.base) {
                Some(t) => t.clone(),
                None => {
                    return self.fail(format!(
                        "в записанном состоянии {}: нет переменной {}",
                        text, p.base
                    ))
                }
            };
            let mut steps = Vec::new();
            for seg in &p.path {
                let next = match (seg, &ty) {
                    (Seg::Field(f), Type::Named(n)) => self
                        .d
                        .structs
                        .get(n.as_str())
                        .and_then(|s| s.fields.iter().position(|x| x.name == *f).map(|k| (s, k)))
                        .map(|(s, k)| (Step::Field(k), s.fields[k].ty.clone())),
                    (Seg::Pos(k), Type::Tuple(ts)) => ts
                        .get(*k as usize)
                        .map(|t| (Step::Pos(*k as usize), t.clone())),
                    _ => None,
                };
                match next {
                    Some((step, t)) => {
                        steps.push(step);
                        ty = t;
                    }
                    None => {
                        return self.fail(format!(
                            "в записанном состоянии {}: называются переменные, поля записей и элементы кортежей",
                            text
                        ))
                    }
                }
            }
            if let Some((q, _)) = listed.iter().find(|(q, _)| overlaps(p, q)) {
                return self.fail(format!(
                    "в записанном состоянии места {} и {} повторяются или пересекаются",
                    ir_text::place(q),
                    text
                ));
            }
            self.set(&mut st, &p.base, &steps, Held::Full)?;
            listed.push((p, steps));
        }
        for (p, steps) in &listed {
            if let Some((_, parent)) = steps.split_last() {
                if self.get(&st, &p.base, parent) == Held::Full {
                    let mut whole = (*p).clone();
                    whole.path.truncate(parent.len());
                    return self.fail(format!(
                        "в записанном состоянии названы все части {}: пишется само место",
                        ir_text::place(&whole)
                    ));
                }
            }
        }
        Ok(st)
    }

    // ── известные варианты ──

    fn flags(&self) -> Vec<bool> {
        self.facts.iter().map(|f| f.live).collect()
    }

    fn set_flags(&mut self, flags: &[bool]) {
        for (f, live) in self.facts.iter_mut().zip(flags) {
            f.live = *live;
        }
    }

    /// После записи в место или его отдачи вызову для изменения неизвестны варианты мест,
    /// которые оно может содержать, и мест, номера элементов в которых берутся из него.
    fn kill_facts(&mut self, written: &Place) {
        for f in &mut self.facts {
            if may_contain(written, &f.place) || index_vars(&f.place).any(|v| v == written.base) {
                f.live = false;
            }
        }
    }

    // ── места ──

    fn resolve(&self, st: &St, p: &Place) -> R<(Vec<Step>, Type)> {
        self.place_path(st, p, true)
    }

    /// Путь к месту, сверенный с типами, и тип места. Значение варианта берут только там, где
    /// вариант известен. Номер элемента — переменная `i64` или число; если `indices`, у
    /// переменной-номера проверяется значение.
    fn place_path(&self, st: &St, p: &Place, indices: bool) -> R<(Vec<Step>, Type)> {
        let text = ir_text::place(p);
        let mut ty = match self.types.get(&p.base) {
            Some(t) => t.clone(),
            None => return self.fail(format!("нет переменной {}", p.base)),
        };
        let bad = || self.fail(format!("путь {} не подходит к типу", text));
        let mut steps = Vec::new();
        let mut i = 0;
        while i < p.path.len() {
            let (step, next) = match (&p.path[i], &ty) {
                (Seg::Field(f), Type::Named(n)) if self.d.structs.contains_key(n.as_str()) => {
                    let s = self.d.structs[n.as_str()];
                    match s.fields.iter().position(|x| x.name == *f) {
                        Some(k) => (Step::Field(k), s.fields[k].ty.clone()),
                        None => return bad(),
                    }
                }
                (Seg::Field(v), Type::Named(n)) if self.d.enums.contains_key(n.as_str()) => {
                    let e = self.d.enums[n.as_str()];
                    let vi = match e.variants.iter().position(|x| x.name == *v) {
                        Some(vi) => vi,
                        None => return bad(),
                    };
                    let prefix = Place {
                        base: p.base.clone(),
                        path: p.path[..i].to_vec(),
                    };
                    match self.facts.iter().rev().find(|f| f.live && f.place == prefix) {
                        Some(f) if f.variant == vi => {}
                        Some(f) => {
                            return self.fail(format!(
                                "{}: в {} вариант {}",
                                text,
                                ir_text::place(&prefix),
                                e.variants[f.variant].name
                            ))
                        }
                        None => {
                            return self.fail(format!(
                                "{}: вариант в {} здесь неизвестен — значение варианта берут в его ветке case, пока место не переписано",
                                text,
                                ir_text::place(&prefix)
                            ))
                        }
                    }
                    i += 1;
                    match (&e.variants[vi].payload, p.path.get(i)) {
                        (Payload::One(t), Some(Seg::Pos(0))) => (Step::Variant(vi, 0), t.clone()),
                        (Payload::Fields(fs), Some(Seg::Field(name))) => {
                            match fs.iter().position(|x| x.name == *name) {
                                Some(k) => (Step::Variant(vi, k), fs[k].ty.clone()),
                                None => return bad(),
                            }
                        }
                        _ => return bad(),
                    }
                }
                (Seg::Pos(k), Type::Tuple(ts)) if (*k as usize) < ts.len() => {
                    (Step::Pos(*k as usize), ts[*k as usize].clone())
                }
                (Seg::Elem(index), Type::Array(t, _)) => {
                    if let Index::Var(v) = index {
                        if self.types.get(v) != Some(&Type::Scalar(Scalar::I64)) {
                            return self.fail(format!("номер элемента {} — не переменная i64", v));
                        }
                        if indices && self.get(st, v, &[]) != Held::Full {
                            return self.fail(format!("в номере элемента {} нет значения", v));
                        }
                    }
                    (Step::Elem, t.as_ref().clone())
                }
                _ => return bad(),
            };
            steps.push(step);
            ty = next;
            i += 1;
        }
        Ok((steps, ty))
    }

    /// Объявление: у переменной один тип на всю функцию, параметр заново не объявляется.
    /// В переменной не должно остаться непростого значения.
    fn declare(&mut self, st: &mut St, name: &str, ty: &Type) -> R<()> {
        if !name_ok(name) {
            return self.fail(format!(
                "переменная {:?}: имя нельзя записать текстом IR — оно пустое или с обратной кавычкой или управляющим символом",
                name
            ));
        }
        self.check_type(ty)?;
        if self.modes.contains_key(name) {
            return self.fail(format!("{} — параметр, объявить его заново нельзя", name));
        }
        match self.types.get(name) {
            Some(t) if t != ty => {
                return self.fail(format!(
                    "{} уже объявлена с типом {}, а здесь {}",
                    name,
                    ir_text::ty(t),
                    ir_text::ty(ty)
                ))
            }
            Some(_) => {}
            None => {
                self.types.insert(name.to_string(), ty.clone());
            }
        }
        if let Some(h) = st.get(name) {
            if let Some(at) = self.first_owned(h, ty, name.to_string()) {
                return self.fail(format!(
                    "в {} ещё есть непростое значение: сначала drop",
                    at
                ));
            }
        }
        st.remove(name);
        Ok(())
    }

    // ── значения ──

    fn constant(&self, c: &Const, expected: &Type) -> R<()> {
        match (c, expected) {
            (Const::Int(n), Type::Scalar(s)) if s.int_range().is_some() => match s.int_range() {
                Some((lo, hi)) if lo <= *n && *n <= hi => Ok(()),
                _ => self.fail(format!("константа {} не помещается в {}", n, s.name())),
            },
            (Const::Float(bits), Type::Scalar(s @ (Scalar::F32 | Scalar::F64))) => {
                let x = f64::from_bits(*bits);
                if !x.is_finite() {
                    return self.fail("дробная константа должна быть конечной".to_string());
                }
                if *s == Scalar::F32 && ((x as f32) as f64).to_bits() != *bits {
                    return self.fail(format!("константа {:?} — не точное значение f32", x));
                }
                Ok(())
            }
            (Const::Bool(_), Type::Scalar(Scalar::Bool))
            | (Const::Char(_), Type::Scalar(Scalar::Char))
            | (Const::Unit, Type::Unit) => Ok(()),
            _ => self.fail(format!(
                "константа не подходит к типу {}",
                ir_text::ty(expected)
            )),
        }
    }

    fn copy_of(&self, st: &St, p: &Place, expected: Option<&Type>) -> R<Type> {
        let (steps, ty) = self.resolve(st, p)?;
        if let Some(e) = expected {
            self.same_type(&ty, e)?;
        }
        if !self.d.is_copy(&ty) {
            return self.fail(format!(
                "copy {}: тип {} не простой",
                ir_text::place(p),
                ir_text::ty(&ty)
            ));
        }
        if self.get(st, &p.base, &steps) != Held::Full {
            return self.fail(format!(
                "copy {}: значения нет или часть его перемещена",
                ir_text::place(p)
            ));
        }
        Ok(ty)
    }

    fn take(&mut self, st: &mut St, p: &Place, expected: Option<&Type>) -> R<Type> {
        let (steps, ty) = self.resolve(st, p)?;
        if let Some(e) = expected {
            self.same_type(&ty, e)?;
        }
        self.owned(p, "move")?;
        if steps.contains(&Step::Elem) {
            return self.fail(format!(
                "move {}: элемент массива отдельно не забирают — массив держится только целиком",
                ir_text::place(p)
            ));
        }
        if self.get(st, &p.base, &steps) != Held::Full {
            return self.fail(format!(
                "move {}: значения нет или часть его перемещена",
                ir_text::place(p)
            ));
        }
        self.set(st, &p.base, &steps, Held::Empty)?;
        Ok(ty)
    }

    fn operand(&mut self, st: &mut St, o: &Operand, expected: &Type) -> R<()> {
        match o {
            Operand::Const(c) => self.constant(c, expected),
            Operand::Copy(p) => self.copy_of(st, p, Some(expected)).map(|_| ()),
            Operand::Move(p) => self.take(st, p, Some(expected)).map(|_| ()),
            Operand::Borrow(p) | Operand::BorrowMut(p) => self.fail(format!(
                "заимствование {} бывает только аргументом вызова: ссылок как значений на шаге 2 нет",
                ir_text::place(p)
            )),
        }
    }

    /// Величина сдвига: целое любого типа.
    fn shift_count(&mut self, st: &mut St, o: &Operand) -> R<()> {
        let ty = match o {
            Operand::Const(Const::Int(_)) => return Ok(()),
            Operand::Copy(p) => self.copy_of(st, p, None)?,
            Operand::Move(p) => self.take(st, p, None)?,
            _ => return self.fail("величина сдвига — целое".to_string()),
        };
        match ty {
            Type::Scalar(s) if s.int_range().is_some() => Ok(()),
            _ => self.fail(format!(
                "величина сдвига типа {} — не целое",
                ir_text::ty(&ty)
            )),
        }
    }

    fn rhs(&mut self, st: &mut St, r: &Rhs, expected: &Type) -> R<()> {
        match r {
            Rhs::Use(o) => self.operand(st, o, expected),
            Rhs::Op { op, ty, args } => {
                if !op_allowed(*op, *ty) {
                    return self.fail(format!("у типа {} нет операции {}", ty.name(), op.name()));
                }
                if args.len() != op.arity() {
                    return self.fail(format!(
                        "операция {} берёт операндов: {}; дано: {}",
                        op.name(),
                        op.arity(),
                        args.len()
                    ));
                }
                for (i, a) in args.iter().enumerate() {
                    if i == 1 && matches!(op, Op::Shl | Op::Shr) {
                        self.shift_count(st, a)?;
                    } else {
                        self.operand(st, a, &Type::Scalar(*ty))?;
                    }
                }
                let result = match op {
                    Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => Scalar::Bool,
                    _ => *ty,
                };
                self.same_type(&Type::Scalar(result), expected)
            }
            Rhs::Conv { from, to, arg } => {
                if !conv_allowed(*from, *to) {
                    return self.fail(format!(
                        "преобразования conv.{}.{} нет",
                        from.name(),
                        to.name()
                    ));
                }
                self.operand(st, arg, &Type::Scalar(*from))?;
                self.same_type(&Type::Scalar(*to), expected)
            }
            Rhs::Call { func, args } => {
                let f = self.func(func)?;
                if self.d.is_never(&f.result) {
                    return self.fail(format!(
                        "функцию {} с результатом — перечислением без вариантов — вызывают только инструкцией call",
                        func
                    ));
                }
                self.call(st, f, args)?;
                self.same_type(&f.result, expected)
            }
            Rhs::Make { ty, variant, args } => {
                self.same_type(&Type::Named(ty.clone()), expected)?;
                self.make(st, ty, variant.as_deref(), args.as_deref())
            }
            Rhs::Tuple(items) => match expected {
                Type::Tuple(ts) if ts.len() == items.len() => {
                    for (o, t) in items.iter().zip(ts) {
                        self.operand(st, o, t)?;
                    }
                    Ok(())
                }
                _ => self.fail(format!(
                    "кортеж не подходит к типу {}",
                    ir_text::ty(expected)
                )),
            },
            Rhs::Array(items) => match expected {
                Type::Array(t, n) if *n == items.len() as u64 => {
                    for o in items {
                        self.operand(st, o, t)?;
                    }
                    Ok(())
                }
                _ => self.fail(format!(
                    "массив не подходит к типу {}",
                    ir_text::ty(expected)
                )),
            },
        }
    }

    /// Поля по именам в порядке записи; каждое ровно один раз.
    fn named_fields(&mut self, st: &mut St, fields: &[Field], args: &[Arg], what: &str) -> R<()> {
        let mut seen = vec![false; fields.len()];
        for a in args {
            let k = a
                .name
                .as_ref()
                .and_then(|name| fields.iter().position(|f| f.name == *name));
            match k {
                Some(k) if !seen[k] => {
                    seen[k] = true;
                    self.operand(st, &a.value, &fields[k].ty)?;
                }
                _ => return self.fail(format!("{}: поля указаны не так, как объявлены", what)),
            }
        }
        if seen.iter().any(|s| !s) {
            return self.fail(format!("{}: указаны не все поля", what));
        }
        Ok(())
    }

    fn make(
        &mut self,
        st: &mut St,
        ty: &str,
        variant: Option<&str>,
        args: Option<&[Arg]>,
    ) -> R<()> {
        let vname = match variant {
            None => {
                let what = format!("make {}", ty);
                let s: &'m StructDecl = match self.d.structs.get(ty) {
                    Some(s) => s,
                    None => return self.fail(format!("{}: нет такой записи", what)),
                };
                return match args {
                    Some(args) => self.named_fields(st, &s.fields, args, &what),
                    None => self.fail(format!("{}: поля записи — в скобках", what)),
                };
            }
            Some(v) => v,
        };
        let what = format!("make {}.{}", ty, vname);
        let e: &'m EnumDecl = match self.d.enums.get(ty) {
            Some(e) => e,
            None => return self.fail(format!("{}: нет такого перечисления", what)),
        };
        let var = match e.variants.iter().find(|v| v.name == vname) {
            Some(v) => v,
            None => return self.fail(format!("{}: нет такого варианта", what)),
        };
        match (&var.payload, args) {
            (Payload::None, None) => Ok(()),
            (Payload::One(t), Some([a])) if a.name.is_none() => self.operand(st, &a.value, t),
            (Payload::Fields(fs), Some(args)) => self.named_fields(st, fs, args, &what),
            _ => self.fail(format!(
                "{}: значения не подходят к объявлению варианта",
                what
            )),
        }
    }

    /// Обращение к месту, пока предыдущие аргументы вызова держат заимствования: место для
    /// изменения или забираемое не пересекается ни с каким из них, место для чтения — с
    /// заимствованием для изменения. Номера элементов места читаются, поэтому и их переменные
    /// не отданы для изменения.
    fn lent_check(&self, active: &[(Place, bool)], p: &Place, exclusive: bool) -> R<()> {
        for (q, for_mut) in active {
            if (exclusive || *for_mut) && overlaps(p, q) {
                return self.fail(format!(
                    "{} пересекается с {}, уже отданным заимствованием в этом вызове",
                    ir_text::place(p),
                    ir_text::place(q)
                ));
            }
            if *for_mut && index_vars(p).any(|v| v == q.base) {
                return self.fail(format!(
                    "номер элемента в {} берётся из {}, уже отданного для изменения в этом вызове",
                    ir_text::place(p),
                    ir_text::place(q)
                ));
            }
        }
        Ok(())
    }

    fn call(&mut self, st: &mut St, f: &'m Func, args: &[Operand]) -> R<()> {
        if args.len() != f.params.len() {
            return self.fail(format!(
                "функция {} берёт аргументов: {}; дано: {}",
                f.name,
                f.params.len(),
                args.len()
            ));
        }
        let mut active: Vec<(Place, bool)> = Vec::new();
        for (p, a) in f.params.iter().zip(args) {
            match (p.mode, a) {
                (Mode::Read, Operand::Borrow(place)) => {
                    self.lent_check(&active, place, false)?;
                    let (steps, ty) = self.resolve(st, place)?;
                    self.same_type(&ty, &p.ty)?;
                    if self.get(st, &place.base, &steps) != Held::Full {
                        return self.fail(format!(
                            "borrow {}: значения нет или часть его перемещена",
                            ir_text::place(place)
                        ));
                    }
                    active.push((place.clone(), false));
                }
                (Mode::Read | Mode::Take, Operand::Copy(place)) => {
                    self.lent_check(&active, place, false)?;
                    self.operand(st, a, &p.ty)?;
                }
                (Mode::Read | Mode::Take, Operand::Const(_)) => self.operand(st, a, &p.ty)?,
                (Mode::Mut, Operand::BorrowMut(place)) => {
                    self.lent_check(&active, place, true)?;
                    let (steps, ty) = self.resolve(st, place)?;
                    self.same_type(&ty, &p.ty)?;
                    if self.modes.get(&place.base) == Some(&Mode::Read) {
                        return self.fail(format!(
                            "borrow mut {}: {} — параметр для чтения",
                            ir_text::place(place),
                            place.base
                        ));
                    }
                    if self.get(st, &place.base, &steps) != Held::Full {
                        return self.fail(format!(
                            "borrow mut {}: значения нет или часть его перемещена",
                            ir_text::place(place)
                        ));
                    }
                    // вызов может поменять вариант в месте
                    self.kill_facts(place);
                    active.push((place.clone(), true));
                }
                (Mode::Take, Operand::Move(place)) => {
                    self.lent_check(&active, place, true)?;
                    self.operand(st, a, &p.ty)?;
                }
                _ => {
                    return self.fail(format!(
                        "вызов {}: аргумент не подходит к режиму параметра {}",
                        f.name, p.name
                    ))
                }
            }
        }
        Ok(())
    }

    // ── инструкции ──

    /// Блок; результат — достижим ли его конец. После безусловного выхода инструкций нет.
    fn block(&mut self, st: &mut St, b: &Block) -> R<bool> {
        let n = b.stmts.len();
        for (i, s) in b.stmts.iter().enumerate() {
            self.path.push((i + 1).to_string());
            let reach = self.stmt(st, s)?;
            self.path.pop();
            if !reach {
                if i + 1 < n {
                    self.path.push((i + 2).to_string());
                    return self.fail(
                        "инструкция недостижима: перед ней выход из блока — return, break, continue, crash, вызов функции с результатом — перечислением без вариантов — или конструкция, из которой нет выхода"
                            .to_string(),
                    );
                }
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn nested(&mut self, label: String, st: &mut St, b: &Block) -> R<bool> {
        self.path.push(label);
        let r = self.block(st, b)?;
        self.path.pop();
        Ok(r)
    }

    /// Слияние после `if` и `switch`: состояние записано ровно тогда, когда конец достижим,
    /// и каждая дошедшая до конца ветка с ним сходится.
    fn join(
        &self,
        st: &mut St,
        after: &Option<State>,
        ends: Vec<(String, St)>,
        what: &str,
    ) -> R<bool> {
        match (after, ends.is_empty()) {
            (None, true) => Ok(false),
            (Some(_), true) => self.fail(format!(
                "после {} записано состояние, а ни одна ветка до конца не доходит",
                what
            )),
            (None, false) => self.fail(format!(
                "ветки {} доходят до конца, а состояние после не записано",
                what
            )),
            (Some(rec), false) => {
                let r = self.record(rec)?;
                for (from, s) in &ends {
                    self.fits_state(s, &r, &format!("в конце ветки {}", from))?;
                }
                *st = r;
                Ok(true)
            }
        }
    }

    /// Известные варианты после ветки, дошедшей до конца: остаются известными, только если
    /// известны в конце каждой такой ветки.
    fn merge_flags(acc: &mut Option<Vec<bool>>, flags: Vec<bool>) {
        match acc {
            None => *acc = Some(flags),
            Some(a) => a.iter_mut().zip(flags).for_each(|(x, y)| *x = *x && y),
        }
    }

    fn stmt(&mut self, st: &mut St, s: &Stmt) -> R<bool> {
        match s {
            Stmt::Local { name, ty } => self.declare(st, name, ty)?,
            Stmt::Define { name, ty, value } => {
                self.check_type(ty)?;
                self.rhs(st, value, ty)?;
                self.declare(st, name, ty)?;
                st.insert(name.clone(), Held::Full);
                self.kill_facts(&Place {
                    base: name.clone(),
                    path: Vec::new(),
                });
            }
            Stmt::Assign { place, value } => self.assign(st, place, value)?,
            Stmt::Drop(place) => {
                let (steps, _) = self.resolve(st, place)?;
                self.owned(place, "drop")?;
                if steps.contains(&Step::Elem) {
                    return self.fail(format!(
                        "drop {}: элемент массива отдельно не уничтожают — массив держится только целиком",
                        ir_text::place(place)
                    ));
                }
                if self.get(st, &place.base, &steps) == Held::Empty {
                    return self.fail(format!("drop {}: значения нет", ir_text::place(place)));
                }
                self.set(st, &place.base, &steps, Held::Empty)?;
            }
            Stmt::Call { func, args } => {
                let f = self.func(func)?;
                let never = self.d.is_never(&f.result);
                if f.result != Type::Unit && !never {
                    return self.fail(format!(
                        "результат вызова {} не используется: инструкция call — для функций с результатом () или перечислением без вариантов",
                        func
                    ));
                }
                self.call(st, f, args)?;
                return Ok(!never);
            }
            Stmt::If {
                cond,
                then,
                otherwise,
                after,
            } => {
                let (steps, ty) = self.resolve(st, cond)?;
                self.same_type(&ty, &Type::Scalar(Scalar::Bool))?;
                if self.get(st, &cond.base, &steps) != Held::Full {
                    return self.fail(format!("в условии {} нет значения", ir_text::place(cond)));
                }
                let saved = self.flags();
                let mut known = None;
                let mut ends = Vec::new();
                for (label, b) in [("then", then), ("else", otherwise)] {
                    self.set_flags(&saved);
                    let mut s = st.clone();
                    if self.nested(label.to_string(), &mut s, b)? {
                        ends.push((label.to_string(), s));
                        Self::merge_flags(&mut known, self.flags());
                    }
                }
                self.set_flags(&known.unwrap_or(saved));
                return self.join(st, after, ends, "if");
            }
            Stmt::Switch {
                on,
                cases,
                otherwise,
                after,
            } => return self.switch(st, on, cases, otherwise.as_ref(), after),
            Stmt::Loop {
                label,
                head,
                body,
                after,
            } => return self.lp(st, label, head, body, after),
            Stmt::Break(l) => {
                let i = match self.loops.iter().rposition(|f| f.label == *l) {
                    Some(i) => i,
                    None => return self.fail(format!("нет цикла {}", l)),
                };
                let after = match (&self.loops[i].after, &self.loops[i].after_text) {
                    (Some(a), _) => a.clone(),
                    (None, Some(text)) => self.record(text)?,
                    (None, None) => {
                        return self.fail(format!(
                            "у цикла {} не записано состояние после: выхода из него нет",
                            l
                        ))
                    }
                };
                self.fits_state(st, &after, &format!("на выходе из цикла {}", l))?;
                self.loops[i].after = Some(after);
                return Ok(false);
            }
            Stmt::Continue(l) => {
                let head = match self.loops.iter().rev().find(|f| f.label == *l) {
                    Some(f) => f.head.clone(),
                    None => return self.fail(format!("нет цикла {}", l)),
                };
                self.fits_state(
                    st,
                    &head,
                    &format!("при переходе к следующему проходу цикла {}", l),
                )?;
                return Ok(false);
            }
            Stmt::Return(value) => {
                match value {
                    None if self.result != Type::Unit => {
                        return self.fail(format!(
                            "return без значения, а результат {}",
                            ir_text::ty(&self.result)
                        ))
                    }
                    None => {}
                    Some(o) => {
                        let r = self.result.clone();
                        self.operand(st, o, &r)?;
                    }
                }
                self.no_owned(st)?;
                return Ok(false);
            }
            Stmt::Crash(_) => return Ok(false),
        }
        Ok(true)
    }

    fn assign(&mut self, st: &mut St, place: &Place, value: &Rhs) -> R<()> {
        let text = ir_text::place(place);
        // номера элементов места читаются до правой части
        let (steps, ty) = self.resolve(st, place)?;
        if self.modes.get(&place.base) == Some(&Mode::Read) {
            return self.fail(format!(
                "запись в {}: {} — параметр для чтения",
                text, place.base
            ));
        }
        self.rhs(st, value, &ty)?;
        // правая часть могла сделать вариант на пути неизвестным: сверка ещё раз, номера уже прочитаны
        self.place_path(st, place, false)?;
        match steps.iter().position(|s| *s == Step::Elem) {
            Some(first) => {
                if self.get(st, &place.base, &steps[..first]) != Held::Full {
                    return self.fail(format!(
                        "запись в {}: массива нет или часть пути к нему перемещена",
                        text
                    ));
                }
                if !self.d.is_copy(&ty) {
                    return self.fail(format!(
                        "запись в {}: непростое значение в элементе массива не заменить — его нельзя отдельно забрать или уничтожить",
                        text
                    ));
                }
            }
            None => {
                for j in 0..steps.len() {
                    if self.get(st, &place.base, &steps[..j]) == Held::Empty {
                        return self.fail(format!(
                            "запись в {}: нет значения, частью которого оно было бы",
                            text
                        ));
                    }
                }
                let old = self.get(st, &place.base, &steps);
                if let Some(at) = self.first_owned(&old, &ty, text.clone()) {
                    return self.fail(format!(
                        "в {} ещё есть непростое значение: сначала drop",
                        at
                    ));
                }
                if self.modes.get(&place.base) != Some(&Mode::Mut) {
                    self.set(st, &place.base, &steps, Held::Full)?;
                }
            }
        }
        self.kill_facts(place);
        Ok(())
    }

    fn switch(
        &mut self,
        st: &mut St,
        on: &Place,
        cases: &[Case],
        otherwise: Option<&Block>,
        after: &Option<State>,
    ) -> R<bool> {
        let text = ir_text::place(on);
        let (steps, ty) = self.resolve(st, on)?;
        let held = self.get(st, &on.base, &steps);
        let e: Option<&'m EnumDecl> = match &ty {
            Type::Named(n) => self.d.enums.get(n.as_str()).copied(),
            _ => None,
        };
        // сколько значений у типа
        let total: i128 = match (&ty, e) {
            (_, Some(e)) => e.variants.len() as i128,
            (Type::Scalar(Scalar::Bool), _) => 2,
            // скалярные значения Unicode: все кодовые точки без суррогатов
            (Type::Scalar(Scalar::Char), _) => 0x11_0000 - 0x800,
            (Type::Scalar(s), _) => match s.int_range() {
                Some((lo, hi)) => hi - lo + 1,
                None => return self.fail(format!("switch по {} типа {} нельзя", text, s.name())),
            },
            _ => {
                return self.fail(format!(
                    "switch по {} типа {} нельзя",
                    text,
                    ir_text::ty(&ty)
                ))
            }
        };
        // у варианта части могут быть перемещены: switch смотрит только вариант
        if held == Held::Empty || (e.is_none() && held != Held::Full) {
            return self.fail(format!("в switch {} нет значения", text));
        }
        let mut labels: Vec<(String, Option<usize>)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for c in cases {
            let (key, vi) = match (&c.label, e, &ty) {
                (CaseLabel::Variant(v), Some(e), _) => {
                    match e.variants.iter().position(|x| x.name == *v) {
                        Some(vi) => (v.clone(), Some(vi)),
                        None => return self.fail(format!("case {}: такого варианта нет", v)),
                    }
                }
                (CaseLabel::Value(Const::Int(n)), None, Type::Scalar(s))
                    if s.int_range().is_some() =>
                {
                    match s.int_range() {
                        Some((lo, hi)) if lo <= *n && *n <= hi => (n.to_string(), None),
                        _ => {
                            return self.fail(format!(
                                "case {}: значение не помещается в {}",
                                n,
                                s.name()
                            ))
                        }
                    }
                }
                (CaseLabel::Value(Const::Char(ch)), None, Type::Scalar(Scalar::Char)) => {
                    (format!("{:?}", ch), None)
                }
                (CaseLabel::Value(Const::Bool(b)), None, Type::Scalar(Scalar::Bool)) => {
                    (b.to_string(), None)
                }
                _ => return self.fail(format!("метка case не подходит к switch {}", text)),
            };
            if !seen.insert(key.clone()) {
                return self.fail(format!("case {} повторён", key));
            }
            labels.push((format!("case {}", key), vi));
        }
        let complete = labels.len() as i128 == total;
        match (otherwise.is_some(), complete) {
            (true, true) => {
                return self.fail(format!(
                    "else в switch {} недостижим: ветки покрывают все значения",
                    text
                ))
            }
            (false, false) => {
                return self.fail(format!(
                    "switch {} неполон: ветки покрывают не все значения, а else нет",
                    text
                ))
            }
            _ => {}
        }
        let saved = self.flags();
        let mut known = None;
        let mut ends = Vec::new();
        for (c, (label, vi)) in cases.iter().zip(labels) {
            self.set_flags(&saved);
            let mark = self.facts.len();
            if let Some(vi) = vi {
                self.facts.push(Fact {
                    place: on.clone(),
                    variant: vi,
                    live: true,
                });
            }
            let mut s = st.clone();
            let reach = self.nested(label.clone(), &mut s, &c.body)?;
            self.facts.truncate(mark);
            if reach {
                ends.push((label, s));
                Self::merge_flags(&mut known, self.flags());
            }
        }
        if let Some(b) = otherwise {
            self.set_flags(&saved);
            let mut s = st.clone();
            if self.nested("else".to_string(), &mut s, b)? {
                ends.push(("else".to_string(), s));
                Self::merge_flags(&mut known, self.flags());
            }
        }
        self.set_flags(&known.unwrap_or(saved));
        self.join(st, after, ends, "switch")
    }

    fn lp(
        &mut self,
        st: &mut St,
        label: &str,
        head: &State,
        body: &Block,
        after: &Option<State>,
    ) -> R<bool> {
        if !name_ok(label) {
            return self.fail(format!(
                "метка цикла {:?}: имя нельзя записать текстом IR — оно пустое или с обратной кавычкой или управляющим символом",
                label
            ));
        }
        if self.loops.iter().any(|f| f.label == label) {
            return self.fail(format!("цикл {} вложен в цикл с той же меткой", label));
        }
        let h = self.record(head)?;
        self.fits_state(st, &h, &format!("на входе в цикл {}", label))?;
        // варианты, которые тело может сделать неизвестными, неизвестны уже в начале прохода
        let mut ws = Vec::new();
        written(body, &mut ws);
        for w in &ws {
            self.kill_facts(w);
        }
        self.loops.push(LoopFrame {
            label: label.to_string(),
            head: h.clone(),
            after_text: after.clone(),
            after: None,
        });
        let name = format!("loop {}", label);
        let mut sb = h.clone();
        if self.nested(name.clone(), &mut sb, body)? {
            self.path.push(name);
            self.fits_state(
                &sb,
                &h,
                &format!("в конце тела цикла {} — перед следующим проходом", label),
            )?;
            self.path.pop();
        }
        let frame = match self.loops.pop() {
            Some(f) => f,
            None => return self.fail("цикл потерян".to_string()),
        };
        match (frame.after, frame.after_text.is_some()) {
            (Some(a), _) => {
                *st = a;
                Ok(true)
            }
            (None, true) => self.fail(format!(
                "после цикла {} записано состояние, а выхода из него нет",
                label
            )),
            (None, false) => Ok(false),
        }
    }
}

impl Held {
    fn borrow_parts(&mut self) -> Option<&mut Vec<Held>> {
        match self {
            Held::Parts(cs) => Some(cs),
            _ => None,
        }
    }
}
