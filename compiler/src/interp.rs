//! Интерпретатор IR — эталон смысла.
//!
//! Исполняет IR буквально, инструкция за инструкцией, по правилам `spec/ir-core.md`, раздел 3.
//! Аварии Nulix — `Stop::Crash`. Если IR нарушает правило, которое должен проверить проверщик,
//! интерпретатор не гадает, а останавливается с `Stop::InvalidIr`. Записанные состояния
//! `[held: …]` он не читает: их сверяет проверщик. Владение он проверяет там, где без этого
//! нельзя исполнять: пустое место, копия непростого значения, запись поверх значения,
//! неотданное значение в конце функции.

use crate::ir::*;
use crate::ir_text;
use std::cmp::Ordering;
use std::collections::HashMap;

// ───────── значения и итоги ─────────

/// Значение во время исполнения.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Целое и его тип: от `i8` до `u64`.
    Int(Scalar, i128),
    F32(f32),
    F64(f64),
    Bool(bool),
    Char(char),
    Unit,
    Tuple(Vec<Slot>),
    Array(Vec<Slot>),
    /// Запись: имя типа и поля в порядке объявления.
    Struct(String, Vec<Slot>),
    /// Вариант: имя типа, номер варианта и его значения в порядке объявления.
    Enum(String, usize, Vec<Slot>),
}

/// Место для значения; `None` — значения нет: не положено, перемещено или уничтожено.
pub type Slot = Option<Value>;

/// Почему исполнение остановилось.
#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    /// Авария Nulix.
    Crash(String),
    /// IR нарушает правило, которое проверяет проверщик.
    InvalidIr(String),
    /// Исчерпан бюджет шагов.
    OutOfSteps,
}

/// Пределы исполнения — настройки интерпретатора, а не правила языка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Сколько шагов можно сделать. Шаг — одна выполненная инструкция; каждый следующий проход
    /// цикла — тоже шаг.
    pub steps: u64,
    /// Наибольшая глубина вызовов; глубже — авария «переполнение стека».
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            steps: 10_000_000,
            depth: 1000,
        }
    }
}

/// Итог теста: `stop` пуст, если тест прошёл.
#[derive(Debug, Clone, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub stop: Option<Stop>,
}

fn crash<T>(message: String) -> Result<T, Stop> {
    Err(Stop::Crash(message))
}

fn invalid<T>(message: String) -> Result<T, Stop> {
    Err(Stop::InvalidIr(message))
}

// ───────── входы ─────────

/// Выполняет тесты модуля по порядку, каждый отдельно и со своим бюджетом шагов. Ошибка —
/// если модуль нельзя исполнять совсем: другая версия IR, имя объявлено дважды.
pub fn run_tests(m: &Module, limits: Limits) -> Result<Vec<TestResult>, Stop> {
    let m = m.clone();
    crate::stack::run(move || {
        let prog = Program::new(&m)?;
        let mut results = Vec::new();
        for d in &m.decls {
            if let Decl::Test(t) = d {
                let mut machine = Machine::new(&prog, limits);
                let stop = machine.run_test(t).err();
                results.push(TestResult {
                    name: t.name.clone(),
                    stop,
                });
            }
        }
        Ok(results)
    })
}

/// Вызывает функцию с готовыми аргументами и возвращает её результат. Параметры `mut` здесь
/// не поддерживаются: изменённое значение некуда вернуть.
pub fn call(m: &Module, func: &str, args: Vec<Value>, limits: Limits) -> Result<Value, Stop> {
    let m = m.clone();
    let func = func.to_string();
    crate::stack::run(move || {
        let prog = Program::new(&m)?;
        let mut machine = Machine::new(&prog, limits);
        machine.call_with_values(&func, args)
    })
}

// ───────── числа ─────────

fn int_bits(s: Scalar) -> u32 {
    match s {
        Scalar::I8 | Scalar::U8 => 8,
        Scalar::I16 | Scalar::U16 => 16,
        Scalar::I32 | Scalar::U32 => 32,
        _ => 64,
    }
}

/// Все биты целого типа.
fn mask(s: Scalar) -> i128 {
    (1i128 << int_bits(s)) - 1
}

/// Значение в сообщении.
fn show(v: &Value) -> String {
    match v {
        Value::Int(_, n) => n.to_string(),
        Value::F32(x) => format!("{:?}", x),
        Value::F64(x) => format!("{:?}", x),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => format!("{:?}", c),
        Value::Unit => "()".to_string(),
        _ => "…".to_string(),
    }
}

fn compare(op: Op, ord: Ordering) -> Option<bool> {
    Some(match op {
        Op::Eq => ord == Ordering::Equal,
        Op::Ne => ord != Ordering::Equal,
        Op::Lt => ord == Ordering::Less,
        Op::Le => ord != Ordering::Greater,
        Op::Gt => ord == Ordering::Greater,
        Op::Ge => ord != Ordering::Less,
        _ => return None,
    })
}

/// Действия над дробными одного типа; `None` — у дробных такой операции нет.
fn float_op<F>(op: Op, a: F, b: Option<F>, wrap: fn(F) -> Value) -> Option<Value>
where
    F: Copy
        + PartialOrd
        + std::ops::Add<Output = F>
        + std::ops::Sub<Output = F>
        + std::ops::Mul<Output = F>
        + std::ops::Div<Output = F>
        + std::ops::Rem<Output = F>
        + std::ops::Neg<Output = F>,
{
    Some(match op {
        Op::Add => wrap(a + b?),
        Op::Sub => wrap(a - b?),
        Op::Mul => wrap(a * b?),
        Op::Div => wrap(a / b?),
        Op::Rem => wrap(a % b?),
        Op::Neg => wrap(-a),
        // сравнения по IEEE 754: с NaN ложно всё, кроме ne
        Op::Eq => Value::Bool(a == b?),
        Op::Ne => Value::Bool(a != b?),
        Op::Lt => Value::Bool(a < b?),
        Op::Le => Value::Bool(a <= b?),
        Op::Gt => Value::Bool(a > b?),
        Op::Ge => Value::Bool(a >= b?),
        _ => return None,
    })
}

/// Операция с проверкой над готовыми значениями. У `shl` и `shr` второй операнд — целое
/// любого типа; у остальных все операнды типа `ty`.
pub fn eval_op(op: Op, ty: Scalar, args: &[Value]) -> Result<Value, Stop> {
    let text = || {
        let shown: Vec<String> = args.iter().map(show).collect();
        format!("{}.{}({})", op.name(), ty.name(), shown.join(", "))
    };
    if args.len() != op.arity() {
        return invalid(format!(
            "операция {} берёт операндов: {}; дано: {}",
            op.name(),
            op.arity(),
            args.len()
        ));
    }
    let wrong = || invalid(format!("{}: операнды не того типа", text()));
    let no_op = || {
        invalid(format!(
            "{}: у типа {} нет такой операции",
            text(),
            ty.name()
        ))
    };
    if let Some((lo, hi)) = ty.int_range() {
        let a = match &args[0] {
            Value::Int(s, n) if *s == ty => *n,
            _ => return wrong(),
        };
        let fit = |r: Option<i128>| match r {
            Some(r) if lo <= r && r <= hi => Ok(Value::Int(ty, r)),
            _ => crash(format!("переполнение: {}", text())),
        };
        match op {
            Op::Neg if ty.is_signed() => return fit(a.checked_neg()),
            Op::Neg => return no_op(),
            Op::Not if ty.is_signed() => return Ok(Value::Int(ty, !a)),
            Op::Not => return Ok(Value::Int(ty, !a & mask(ty))),
            Op::Shl | Op::Shr => {
                let count = match &args[1] {
                    Value::Int(_, c) => *c,
                    _ => return wrong(),
                };
                let bits = int_bits(ty);
                if count < 0 || count >= bits as i128 {
                    return crash(format!(
                        "сдвиг на {} при ширине {}: {}",
                        count,
                        bits,
                        text()
                    ));
                }
                if op == Op::Shr {
                    // у знаковых арифметический: значение хранится со знаком
                    return Ok(Value::Int(ty, a >> count));
                }
                let m = mask(ty) as u128;
                let r = (((a as u128) & m) << count) & m;
                let r = if ty.is_signed() && (r >> (bits - 1)) & 1 == 1 {
                    r as i128 - (1i128 << bits)
                } else {
                    r as i128
                };
                return Ok(Value::Int(ty, r));
            }
            _ => {}
        }
        let b = match &args[1] {
            Value::Int(s, n) if *s == ty => *n,
            _ => return wrong(),
        };
        return match op {
            Op::Add => fit(a.checked_add(b)),
            Op::Sub => fit(a.checked_sub(b)),
            Op::Mul => fit(a.checked_mul(b)),
            Op::Div | Op::Rem if b == 0 => crash(format!("деление на ноль: {}", text())),
            Op::Div => fit(a.checked_div(b)),
            Op::Rem => fit(a.checked_rem(b)),
            Op::And => Ok(Value::Int(ty, a & b)),
            Op::Or => Ok(Value::Int(ty, a | b)),
            Op::Xor => Ok(Value::Int(ty, a ^ b)),
            _ => match compare(op, a.cmp(&b)) {
                Some(r) => Ok(Value::Bool(r)),
                None => no_op(),
            },
        };
    }
    let result = match (ty, &args[0], args.get(1)) {
        (Scalar::F32, Value::F32(a), None) => float_op(op, *a, None, Value::F32),
        (Scalar::F32, Value::F32(a), Some(Value::F32(b))) => float_op(op, *a, Some(*b), Value::F32),
        (Scalar::F64, Value::F64(a), None) => float_op(op, *a, None, Value::F64),
        (Scalar::F64, Value::F64(a), Some(Value::F64(b))) => float_op(op, *a, Some(*b), Value::F64),
        (Scalar::Bool, Value::Bool(a), None) => match op {
            Op::Not => Some(Value::Bool(!a)),
            _ => None,
        },
        (Scalar::Bool, Value::Bool(a), Some(Value::Bool(b))) => match op {
            Op::And => Some(Value::Bool(*a & *b)),
            Op::Or => Some(Value::Bool(*a | *b)),
            Op::Xor => Some(Value::Bool(*a ^ *b)),
            // у bool нет порядка: только eq и ne
            Op::Eq => Some(Value::Bool(a == b)),
            Op::Ne => Some(Value::Bool(a != b)),
            _ => None,
        },
        (Scalar::Char, Value::Char(a), Some(Value::Char(b))) => {
            compare(op, a.cmp(b)).map(Value::Bool)
        }
        (Scalar::Char, Value::Char(_), None) => None,
        _ => return wrong(),
    };
    match result {
        Some(v) => Ok(v),
        None => no_op(),
    }
}

/// Дробное в целое с отбрасыванием дробной части; `None` — NaN или бесконечность. Диапазон
/// проверяет вызывающий: `as` насыщает значения за пределами i128, а они вне любого типа.
fn float_to_int(x: f64) -> Option<i128> {
    if x.is_finite() {
        Some(x.trunc() as i128)
    } else {
        None
    }
}

/// Преобразование `conv`: диапазон проверяется, точность округляется.
pub fn eval_conv(from: Scalar, to: Scalar, v: &Value) -> Result<Value, Stop> {
    let text = || format!("conv.{}.{}({})", from.name(), to.name(), show(v));
    let source_ok = match (from, v) {
        (s, Value::Int(s2, _)) => s == *s2,
        (Scalar::F32, Value::F32(_)) | (Scalar::F64, Value::F64(_)) => true,
        (Scalar::Char, Value::Char(_)) | (Scalar::Bool, Value::Bool(_)) => true,
        _ => false,
    };
    if !source_ok {
        return invalid(format!("{}: операнд не типа {}", text(), from.name()));
    }
    let not_fit = || crash(format!("не помещается: {}", text()));
    match (v, to) {
        (Value::Bool(_), _) | (_, Scalar::Bool) => {
            invalid(format!("{}: bool ни во что не преобразуется", text()))
        }
        (Value::Char(_), Scalar::F32 | Scalar::F64)
        | (Value::F32(_) | Value::F64(_), Scalar::Char) => invalid(format!(
            "{}: символ и дробное друг в друга не преобразуются",
            text()
        )),
        (Value::Char(c), Scalar::Char) => Ok(Value::Char(*c)),
        (Value::Int(_, n), Scalar::Char) => match u32::try_from(*n).ok().and_then(char::from_u32) {
            Some(c) => Ok(Value::Char(c)),
            None => crash(format!("не символ Unicode: {}", text())),
        },
        // к ближайшему, при равенстве — к чётному
        (Value::Int(_, n), Scalar::F32) => Ok(Value::F32(*n as f32)),
        (Value::Int(_, n), Scalar::F64) => Ok(Value::F64(*n as f64)),
        (Value::F32(x), Scalar::F32) => Ok(Value::F32(*x)),
        (Value::F32(x), Scalar::F64) => Ok(Value::F64(*x as f64)),
        (Value::F64(x), Scalar::F64) => Ok(Value::F64(*x)),
        (Value::F64(x), Scalar::F32) => {
            let r = *x as f32;
            // бесконечность и NaN переходят как есть, конечное за пределами f32 — авария
            if r.is_infinite() && x.is_finite() {
                not_fit()
            } else {
                Ok(Value::F32(r))
            }
        }
        _ => {
            let (lo, hi) = match to.int_range() {
                Some(range) => range,
                None => return invalid(format!("{}: нет такого преобразования", text())),
            };
            let n = match v {
                Value::Int(_, n) => Some(*n),
                Value::Char(c) => Some(*c as u32 as i128),
                Value::F32(x) => float_to_int(*x as f64),
                Value::F64(x) => float_to_int(*x),
                _ => None,
            };
            match n {
                Some(n) if lo <= n && n <= hi => Ok(Value::Int(to, n)),
                _ => not_fit(),
            }
        }
    }
}

// ───────── модуль ─────────

struct Program<'m> {
    structs: HashMap<&'m str, &'m StructDecl>,
    enums: HashMap<&'m str, &'m EnumDecl>,
    funcs: HashMap<&'m str, &'m Func>,
}

fn payload_types(p: &Payload) -> Vec<&Type> {
    match p {
        Payload::None => Vec::new(),
        Payload::One(t) => vec![t],
        Payload::Fields(fs) => fs.iter().map(|f| &f.ty).collect(),
    }
}

/// Все части значения на месте.
fn is_complete(v: &Value) -> bool {
    match v {
        Value::Tuple(xs) | Value::Array(xs) | Value::Struct(_, xs) | Value::Enum(_, _, xs) => {
            xs.iter().all(|x| x.as_ref().is_some_and(is_complete))
        }
        _ => true,
    }
}

impl<'m> Program<'m> {
    fn new(m: &'m Module) -> Result<Program<'m>, Stop> {
        if m.version != VERSION {
            return invalid(format!("версия IR {} не поддерживается", m.version));
        }
        let mut p = Program {
            structs: HashMap::new(),
            enums: HashMap::new(),
            funcs: HashMap::new(),
        };
        for d in &m.decls {
            match d {
                Decl::Struct(s) => {
                    if p.is_type(&s.name) {
                        return invalid(format!("тип {} объявлен дважды", s.name));
                    }
                    p.structs.insert(&s.name, s);
                }
                Decl::Enum(e) => {
                    if p.is_type(&e.name) {
                        return invalid(format!("тип {} объявлен дважды", e.name));
                    }
                    p.enums.insert(&e.name, e);
                }
                Decl::Func(f) => {
                    if p.funcs.insert(&f.name, f).is_some() {
                        return invalid(format!("функция {} объявлена дважды", f.name));
                    }
                }
                Decl::Test(_) => {}
            }
        }
        Ok(p)
    }

    fn is_type(&self, name: &str) -> bool {
        self.structs.contains_key(name) || self.enums.contains_key(name)
    }

    /// Тип записан правильно: все имена типов объявлены.
    fn check_type(&self, t: &Type) -> Result<(), Stop> {
        match t {
            Type::Scalar(_) | Type::Unit => Ok(()),
            Type::Tuple(ts) => ts.iter().try_for_each(|t| self.check_type(t)),
            Type::Array(t, _) => self.check_type(t),
            Type::Named(n) if self.is_type(n) => Ok(()),
            Type::Named(n) => invalid(format!("нет типа {}", n)),
        }
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

    fn is_never(&self, t: &Type) -> bool {
        match t {
            Type::Named(n) => self
                .enums
                .get(n.as_str())
                .is_some_and(|e| e.variants.is_empty()),
            _ => false,
        }
    }

    /// Значение целиком подходит к типу: все части на месте и нужных типов.
    fn conforms(&self, v: &Value, t: &Type) -> bool {
        match (v, t) {
            (Value::Int(s, n), Type::Scalar(s2)) => {
                s == s2 && s.int_range().is_some_and(|(lo, hi)| lo <= *n && *n <= hi)
            }
            (Value::F32(_), Type::Scalar(Scalar::F32))
            | (Value::F64(_), Type::Scalar(Scalar::F64))
            | (Value::Bool(_), Type::Scalar(Scalar::Bool))
            | (Value::Char(_), Type::Scalar(Scalar::Char))
            | (Value::Unit, Type::Unit) => true,
            (Value::Tuple(xs), Type::Tuple(ts)) => {
                xs.len() == ts.len() && xs.iter().zip(ts).all(|(x, t)| self.slot_conforms(x, t))
            }
            (Value::Array(xs), Type::Array(t, n)) => {
                xs.len() as u64 == *n && xs.iter().all(|x| self.slot_conforms(x, t))
            }
            (Value::Struct(name, xs), Type::Named(n)) => {
                name == n
                    && self.structs.get(n.as_str()).is_some_and(|s| {
                        s.fields.len() == xs.len()
                            && xs
                                .iter()
                                .zip(&s.fields)
                                .all(|(x, f)| self.slot_conforms(x, &f.ty))
                    })
            }
            (Value::Enum(name, vi, xs), Type::Named(n)) => {
                name == n
                    && self
                        .enums
                        .get(n.as_str())
                        .and_then(|e| e.variants.get(*vi))
                        .is_some_and(|var| {
                            let ts = payload_types(&var.payload);
                            ts.len() == xs.len()
                                && xs.iter().zip(ts).all(|(x, t)| self.slot_conforms(x, t))
                        })
            }
            _ => false,
        }
    }

    fn slot_conforms(&self, s: &Slot, t: &Type) -> bool {
        s.as_ref().is_some_and(|v| self.conforms(v, t))
    }

    /// Осталось ли в значении то, что нужно отдать или уничтожить. У значения без перемещённых
    /// частей это так, если тип не простой; у частично перемещённого считаются оставшиеся части.
    fn owns(&self, v: &Value, t: &Type) -> bool {
        if is_complete(v) {
            return !self.is_copy(t);
        }
        let any = |xs: &[Slot], ts: Vec<&Type>| {
            xs.iter()
                .zip(ts)
                .any(|(x, t)| x.as_ref().is_some_and(|x| self.owns(x, t)))
        };
        match (v, t) {
            (Value::Tuple(xs), Type::Tuple(ts)) => any(xs, ts.iter().collect()),
            (Value::Array(xs), Type::Array(t, _)) => any(xs, vec![t.as_ref(); xs.len()]),
            (Value::Struct(_, xs), Type::Named(n)) => match self.structs.get(n.as_str()) {
                Some(s) => any(xs, s.fields.iter().map(|f| &f.ty).collect()),
                None => true,
            },
            (Value::Enum(_, vi, xs), Type::Named(n)) => {
                match self.enums.get(n.as_str()).and_then(|e| e.variants.get(*vi)) {
                    Some(var) => any(xs, payload_types(&var.payload)),
                    None => true,
                }
            }
            _ => true,
        }
    }
}

// ───────── исполнение ─────────

/// Переменная функции: тип, режим параметра (у локальных его нет) и значение.
struct Var {
    ty: Type,
    mode: Option<Mode>,
    slot: Slot,
}

struct Frame {
    /// Что исполняется — для сообщений.
    what: String,
    result: Type,
    vars: HashMap<String, Var>,
}

enum Flow {
    Next,
    Break(String),
    Continue(String),
    Return(Value),
}

/// Шаг пути к месту, уже сверенный с типами.
#[derive(Clone, Copy)]
enum Step {
    /// Поле записи.
    Field(usize),
    /// Элемент кортежа.
    Pos(usize),
    /// Элемент массива; номер проверяется при обращении.
    Elem(i128),
    /// Значение варианта: номер варианта и номер значения в нём.
    Variant(usize, usize),
}

/// Что смотрит `switch` и `if`.
enum Key {
    Variant(usize),
    Int(i128),
    Char(char),
    Bool(bool),
}

/// Имена и типы всех переменных тела: из `local` и определений, по порядку записи.
fn collect_locals(b: &Block, out: &mut Vec<(String, Type)>) {
    for s in &b.stmts {
        match s {
            Stmt::Local { name, ty } | Stmt::Define { name, ty, .. } => {
                out.push((name.clone(), ty.clone()))
            }
            Stmt::If {
                then, otherwise, ..
            } => {
                collect_locals(then, out);
                collect_locals(otherwise, out);
            }
            Stmt::Switch {
                cases, otherwise, ..
            } => {
                for c in cases {
                    collect_locals(&c.body, out);
                }
                if let Some(b) = otherwise {
                    collect_locals(b, out);
                }
            }
            Stmt::Loop { body, .. } => collect_locals(body, out),
            _ => {}
        }
    }
}

/// Спуск по пути к месту. Номер элемента вне массива — авария.
fn walk<'a>(mut cur: &'a mut Slot, steps: &[Step], p: &Place) -> Result<&'a mut Slot, Stop> {
    for step in steps {
        let value = match cur {
            Some(v) => v,
            None => return invalid(format!("на пути к {} нет значения", ir_text::place(p))),
        };
        cur = match (*step, value) {
            (Step::Field(k), Value::Struct(_, xs)) | (Step::Pos(k), Value::Tuple(xs)) => {
                match xs.get_mut(k) {
                    Some(s) => s,
                    None => return invalid(format!("в {} нет такой части", ir_text::place(p))),
                }
            }
            (Step::Elem(n), Value::Array(xs)) => {
                let len = xs.len();
                if n < 0 || n >= len as i128 {
                    return crash(format!("выход за границы: индекс {}, длина {}", n, len));
                }
                &mut xs[n as usize]
            }
            (Step::Variant(vi, k), Value::Enum(_, current, xs)) => {
                if *current != vi {
                    return invalid(format!("в {} другой вариант", ir_text::place(p)));
                }
                match xs.get_mut(k) {
                    Some(s) => s,
                    None => return invalid(format!("в {} нет такой части", ir_text::place(p))),
                }
            }
            _ => {
                return invalid(format!(
                    "значение на пути к {} не того вида",
                    ir_text::place(p)
                ))
            }
        };
    }
    Ok(cur)
}

struct Machine<'p, 'm> {
    prog: &'p Program<'m>,
    steps: u64,
    depth: usize,
    max_depth: usize,
}

impl<'p, 'm> Machine<'p, 'm> {
    fn new(prog: &'p Program<'m>, limits: Limits) -> Self {
        Machine {
            prog,
            steps: limits.steps,
            depth: 0,
            max_depth: limits.depth,
        }
    }

    fn tick(&mut self) -> Result<(), Stop> {
        if self.steps == 0 {
            return Err(Stop::OutOfSteps);
        }
        self.steps -= 1;
        Ok(())
    }

    fn func(&self, name: &str) -> Result<&'m Func, Stop> {
        match self.prog.funcs.get(name) {
            Some(f) => Ok(*f),
            None => invalid(format!("нет функции {}", name)),
        }
    }

    /// Переменные функции заводятся при входе, пустыми: у переменной одно место и один тип
    /// на всю функцию, на каком бы пути её ни объявили.
    fn with_locals(
        &self,
        what: &str,
        body: &Block,
        mut vars: HashMap<String, Var>,
    ) -> Result<HashMap<String, Var>, Stop> {
        let mut locals = Vec::new();
        collect_locals(body, &mut locals);
        for (name, ty) in locals {
            self.prog.check_type(&ty)?;
            match vars.get(&name) {
                Some(var) if var.mode.is_some() => {
                    return invalid(format!(
                        "{}: {} — параметр, объявить его заново нельзя",
                        what, name
                    ))
                }
                Some(var) if var.ty != ty => {
                    return invalid(format!(
                        "{}: {} объявлена с типом {}, а здесь {}",
                        what,
                        name,
                        ir_text::ty(&var.ty),
                        ir_text::ty(&ty)
                    ))
                }
                Some(_) => {}
                None => {
                    let var = Var {
                        ty,
                        mode: None,
                        slot: None,
                    };
                    vars.insert(name, var);
                }
            }
        }
        Ok(vars)
    }

    fn run_test(&mut self, t: &Test) -> Result<(), Stop> {
        let what = format!("тест «{}»", t.name);
        let vars = self.with_locals(&what, &t.body, HashMap::new())?;
        let mut frame = Frame {
            what,
            result: Type::Unit,
            vars,
        };
        let flow = self.block(&mut frame, &t.body)?;
        self.finish(&frame, flow).map(|_| ())
    }

    fn call_with_values(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Stop> {
        let f = self.func(name)?;
        if args.len() != f.params.len() {
            return invalid(format!(
                "функция {} берёт аргументов: {}; дано: {}",
                f.name,
                f.params.len(),
                args.len()
            ));
        }
        let mut vars = HashMap::new();
        for (p, v) in f.params.iter().zip(args) {
            self.prog.check_type(&p.ty)?;
            if p.mode == Mode::Mut {
                return invalid(format!("параметр mut {} здесь не поддерживается", p.name));
            }
            if !self.prog.conforms(&v, &p.ty) {
                return invalid(format!(
                    "аргумент {} не подходит к параметру {}",
                    show(&v),
                    p.name
                ));
            }
            let var = Var {
                ty: p.ty.clone(),
                mode: Some(p.mode),
                slot: Some(v),
            };
            if vars.insert(p.name.clone(), var).is_some() {
                return invalid(format!("параметр {} повторён", p.name));
            }
        }
        self.invoke(f, vars).map(|(v, _)| v)
    }

    /// Исполняет тело функции с готовыми параметрами.
    fn invoke(&mut self, f: &'m Func, vars: HashMap<String, Var>) -> Result<(Value, Frame), Stop> {
        if self.depth >= self.max_depth {
            return crash(format!(
                "переполнение стека: глубина вызовов больше {}",
                self.max_depth
            ));
        }
        let what = format!("функция {}", f.name);
        let vars = self.with_locals(&what, &f.body, vars)?;
        self.depth += 1;
        let mut frame = Frame {
            what,
            result: f.result.clone(),
            vars,
        };
        let flow = self.block(&mut frame, &f.body);
        self.depth -= 1;
        let value = self.finish(&frame, flow?)?;
        Ok((value, frame))
    }

    /// Конец тела: результат и проверка, что каждое значение отдано или уничтожено.
    fn finish(&self, frame: &Frame, flow: Flow) -> Result<Value, Stop> {
        let value = match flow {
            Flow::Return(v) => v,
            Flow::Next if frame.result == Type::Unit => Value::Unit,
            Flow::Next => {
                return invalid(format!(
                    "{}: конец тела достигнут, а результат {}",
                    frame.what,
                    ir_text::ty(&frame.result)
                ))
            }
            Flow::Break(l) | Flow::Continue(l) => {
                return invalid(format!("{}: нет цикла {}", frame.what, l))
            }
        };
        let mut names: Vec<&String> = frame.vars.keys().collect();
        names.sort();
        for name in names {
            let var = &frame.vars[name];
            if matches!(var.mode, Some(Mode::Read) | Some(Mode::Mut)) {
                continue;
            }
            if let Some(v) = &var.slot {
                if self.prog.owns(v, &var.ty) {
                    return invalid(format!(
                        "{}: значение в {} не отдано и не уничтожено",
                        frame.what, name
                    ));
                }
            }
        }
        Ok(value)
    }

    fn block(&mut self, frame: &mut Frame, b: &Block) -> Result<Flow, Stop> {
        for s in &b.stmts {
            let flow = self.stmt(frame, s)?;
            if !matches!(flow, Flow::Next) {
                return Ok(flow);
            }
        }
        Ok(Flow::Next)
    }

    fn stmt(&mut self, frame: &mut Frame, s: &Stmt) -> Result<Flow, Stop> {
        self.tick()?;
        match s {
            Stmt::Local { name, ty } => self.declare(frame, name, ty)?,
            Stmt::Define { name, ty, value } => {
                self.prog.check_type(ty)?;
                let v = self.rhs(frame, value, ty)?;
                self.declare(frame, name, ty)?;
                if let Some(var) = frame.vars.get_mut(name) {
                    var.slot = Some(v);
                }
            }
            Stmt::Assign { place, value } => {
                // номера элементов читаются до правой части, границы проверяются при записи
                let (steps, ty) = self.resolve(frame, place, true)?;
                let v = self.rhs(frame, value, &ty)?;
                self.write(frame, place, &steps, &ty, v)?;
            }
            Stmt::Drop(place) => {
                let (steps, _) = self.resolve(frame, place, true)?;
                let var = self.owned_var(frame, place, "drop")?;
                let slot = walk(&mut var.slot, &steps, place)?;
                if slot.is_none() {
                    return invalid(format!("drop {}: значения нет", ir_text::place(place)));
                }
                *slot = None;
            }
            Stmt::Call { func, args } => {
                let f = self.func(func)?;
                if f.result != Type::Unit && !self.prog.is_never(&f.result) {
                    return invalid(format!(
                        "результат вызова {} не используется; инструкция call — для функций с результатом ()",
                        func
                    ));
                }
                self.call(frame, func, args)?;
            }
            Stmt::If {
                cond,
                then,
                otherwise,
                ..
            } => {
                return match self.inspect(frame, cond)? {
                    Key::Bool(true) => self.block(frame, then),
                    Key::Bool(false) => self.block(frame, otherwise),
                    _ => invalid(format!("условие if {} — не bool", ir_text::place(cond))),
                };
            }
            Stmt::Switch {
                on,
                cases,
                otherwise,
                ..
            } => {
                let key = self.inspect(frame, on)?;
                let (_, ty) = self.resolve(frame, on, false)?;
                for c in cases {
                    if self.matches(&key, &ty, &c.label)? {
                        return self.block(frame, &c.body);
                    }
                }
                return match otherwise {
                    Some(b) => self.block(frame, b),
                    None => invalid(format!(
                        "в switch {} нет ветки для этого значения",
                        ir_text::place(on)
                    )),
                };
            }
            Stmt::Loop { label, body, .. } => loop {
                match self.block(frame, body)? {
                    Flow::Next => {}
                    Flow::Continue(l) if l == *label => {}
                    Flow::Break(l) if l == *label => return Ok(Flow::Next),
                    other => return Ok(other),
                }
                // следующий проход — тоже шаг: иначе пустой цикл обходил бы бюджет
                self.tick()?;
            },
            Stmt::Break(l) => return Ok(Flow::Break(l.clone())),
            Stmt::Continue(l) => return Ok(Flow::Continue(l.clone())),
            Stmt::Return(None) => {
                if frame.result != Type::Unit {
                    return invalid(format!(
                        "{}: return без значения, а результат {}",
                        frame.what,
                        ir_text::ty(&frame.result)
                    ));
                }
                return Ok(Flow::Return(Value::Unit));
            }
            Stmt::Return(Some(o)) => {
                let ty = frame.result.clone();
                let v = self.operand(frame, o, &ty)?;
                return Ok(Flow::Return(v));
            }
            Stmt::Crash(message) => return crash(message.clone()),
        }
        Ok(Flow::Next)
    }

    /// `local` и определение: переменная объявлена один раз с одним типом; повтор — только
    /// когда в ней нет значения, которое надо отдать или уничтожить.
    fn declare(&self, frame: &mut Frame, name: &str, ty: &Type) -> Result<(), Stop> {
        self.prog.check_type(ty)?;
        match frame.vars.get_mut(name) {
            Some(var) => {
                if var.mode.is_some() {
                    return invalid(format!("{} — параметр, объявить его заново нельзя", name));
                }
                if var.ty != *ty {
                    return invalid(format!(
                        "{} объявлена с типом {}, а здесь {}",
                        name,
                        ir_text::ty(&var.ty),
                        ir_text::ty(ty)
                    ));
                }
                if let Some(v) = &var.slot {
                    if self.prog.owns(v, &var.ty) {
                        return invalid(format!("в {} ещё есть значение: сначала drop", name));
                    }
                }
                var.slot = None;
            }
            None => {
                let var = Var {
                    ty: ty.clone(),
                    mode: None,
                    slot: None,
                };
                frame.vars.insert(name.to_string(), var);
            }
        }
        Ok(())
    }

    /// Путь к месту, сверенный с типами, и тип места. Номера элементов читаются, только если
    /// `read`: при записи — после вычисления правой части.
    fn resolve(&self, frame: &Frame, p: &Place, read: bool) -> Result<(Vec<Step>, Type), Stop> {
        let var = match frame.vars.get(&p.base) {
            Some(var) => var,
            None => return invalid(format!("{}: нет переменной {}", frame.what, p.base)),
        };
        let bad_path = || {
            invalid(format!(
                "{}: путь {} не подходит к типу",
                frame.what,
                ir_text::place(p)
            ))
        };
        let mut ty = var.ty.clone();
        let mut steps = Vec::new();
        let mut i = 0;
        while i < p.path.len() {
            let (step, next) = match (&p.path[i], &ty) {
                (Seg::Field(f), Type::Named(n)) => {
                    if let Some(s) = self.prog.structs.get(n.as_str()) {
                        match s.fields.iter().position(|x| x.name == *f) {
                            Some(k) => (Step::Field(k), s.fields[k].ty.clone()),
                            None => return bad_path(),
                        }
                    } else if let Some(e) = self.prog.enums.get(n.as_str()) {
                        let vi = match e.variants.iter().position(|v| v.name == *f) {
                            Some(vi) => vi,
                            None => return bad_path(),
                        };
                        i += 1;
                        match (&e.variants[vi].payload, p.path.get(i)) {
                            (Payload::One(t), Some(Seg::Pos(0))) => {
                                (Step::Variant(vi, 0), t.clone())
                            }
                            (Payload::Fields(fs), Some(Seg::Field(name))) => {
                                match fs.iter().position(|x| x.name == *name) {
                                    Some(k) => (Step::Variant(vi, k), fs[k].ty.clone()),
                                    None => return bad_path(),
                                }
                            }
                            _ => return bad_path(),
                        }
                    } else {
                        return bad_path();
                    }
                }
                (Seg::Pos(k), Type::Tuple(ts)) if (*k as usize) < ts.len() => {
                    (Step::Pos(*k as usize), ts[*k as usize].clone())
                }
                (Seg::Elem(index), Type::Array(t, _)) => {
                    let n = match index {
                        Index::Lit(n) => *n as i128,
                        Index::Var(_) if !read => 0,
                        Index::Var(v) => match frame.vars.get(v).and_then(|x| x.slot.as_ref()) {
                            Some(Value::Int(Scalar::I64, n)) => *n,
                            _ => {
                                return invalid(format!(
                                    "{}: индекс {} — не переменная i64 со значением",
                                    frame.what, v
                                ))
                            }
                        },
                    };
                    (Step::Elem(n), t.as_ref().clone())
                }
                _ => return bad_path(),
            };
            steps.push(step);
            ty = next;
            i += 1;
        }
        Ok((steps, ty))
    }

    /// Переменная, из которой можно забирать значение: локальная или забранный параметр.
    fn owned_var<'f>(
        &self,
        frame: &'f mut Frame,
        p: &Place,
        what: &str,
    ) -> Result<&'f mut Var, Stop> {
        let var = match frame.vars.get_mut(&p.base) {
            Some(var) => var,
            None => return invalid(format!("нет переменной {}", p.base)),
        };
        if matches!(var.mode, Some(Mode::Read) | Some(Mode::Mut)) {
            return invalid(format!(
                "{} {}: параметр {} функции не принадлежит",
                what,
                ir_text::place(p),
                p.base
            ));
        }
        Ok(var)
    }

    /// Значение целиком, без перемещённых частей.
    fn whole(slot: &Slot, p: &Place) -> Result<Value, Stop> {
        match slot {
            Some(v) if is_complete(v) => Ok(v.clone()),
            Some(_) => invalid(format!("в {} часть значения перемещена", ir_text::place(p))),
            None => invalid(format!("в {} нет значения", ir_text::place(p))),
        }
    }

    fn copy_of(&self, frame: &mut Frame, p: &Place) -> Result<Value, Stop> {
        let (steps, ty) = self.resolve(frame, p, true)?;
        if !self.prog.is_copy(&ty) {
            return invalid(format!(
                "copy {}: тип {} не простой",
                ir_text::place(p),
                ir_text::ty(&ty)
            ));
        }
        let var = match frame.vars.get_mut(&p.base) {
            Some(var) => var,
            None => return invalid(format!("нет переменной {}", p.base)),
        };
        Self::whole(walk(&mut var.slot, &steps, p)?, p)
    }

    /// Значение для параметра чтения, переданное заимствованием: вызывающий его не теряет.
    fn lend_read(&self, frame: &mut Frame, p: &Place) -> Result<Value, Stop> {
        let (steps, _) = self.resolve(frame, p, true)?;
        let var = match frame.vars.get_mut(&p.base) {
            Some(var) => var,
            None => return invalid(format!("нет переменной {}", p.base)),
        };
        Self::whole(walk(&mut var.slot, &steps, p)?, p)
    }

    /// Забирает значение из места целиком: `move`, а для `borrow mut` — на время вызова.
    /// Возвращает и путь, по которому взято: на него значение `mut` вернётся.
    fn take_from(
        &self,
        frame: &mut Frame,
        p: &Place,
        mutable: bool,
    ) -> Result<(Value, Vec<Step>), Stop> {
        let (steps, _) = self.resolve(frame, p, true)?;
        let var = if mutable {
            match frame.vars.get_mut(&p.base) {
                Some(var) if var.mode == Some(Mode::Read) => {
                    return invalid(format!(
                        "borrow mut {}: {} — параметр для чтения",
                        ir_text::place(p),
                        p.base
                    ))
                }
                Some(var) => var,
                None => return invalid(format!("нет переменной {}", p.base)),
            }
        } else {
            self.owned_var(frame, p, "move")?
        };
        let slot = walk(&mut var.slot, &steps, p)?;
        let v = Self::whole(slot, p)?;
        *slot = None;
        Ok((v, steps))
    }

    /// Запись в место по уже прочитанному пути: правая часть вычислена.
    fn write(
        &self,
        frame: &mut Frame,
        p: &Place,
        steps: &[Step],
        ty: &Type,
        v: Value,
    ) -> Result<(), Stop> {
        let var = match frame.vars.get_mut(&p.base) {
            Some(var) if var.mode == Some(Mode::Read) => {
                return invalid(format!(
                    "запись в {}: {} — параметр для чтения",
                    ir_text::place(p),
                    p.base
                ))
            }
            Some(var) => var,
            None => return invalid(format!("нет переменной {}", p.base)),
        };
        let slot = walk(&mut var.slot, steps, p)?;
        if let Some(old) = slot {
            if self.prog.owns(old, ty) {
                return invalid(format!(
                    "в {} ещё есть значение: сначала drop",
                    ir_text::place(p)
                ));
            }
        }
        if !self.prog.conforms(&v, ty) {
            return invalid(format!(
                "значение {} не подходит к {}",
                show(&v),
                ir_text::place(p)
            ));
        }
        *slot = Some(v);
        Ok(())
    }

    /// Возвращает значение параметра `mut` на то место, откуда оно было взято.
    fn give_back(
        &self,
        frame: &mut Frame,
        p: &Place,
        steps: &[Step],
        v: Value,
    ) -> Result<(), Stop> {
        let var = match frame.vars.get_mut(&p.base) {
            Some(var) => var,
            None => return invalid(format!("нет переменной {}", p.base)),
        };
        let slot = walk(&mut var.slot, steps, p)?;
        if slot.is_some() {
            return invalid(format!(
                "место {} занято во время вызова",
                ir_text::place(p)
            ));
        }
        *slot = Some(v);
        Ok(())
    }

    /// Что видят `if` и `switch`: вариант или значение скаляра; значение не забирается.
    fn inspect(&self, frame: &mut Frame, p: &Place) -> Result<Key, Stop> {
        let (steps, _) = self.resolve(frame, p, true)?;
        let var = match frame.vars.get_mut(&p.base) {
            Some(var) => var,
            None => return invalid(format!("нет переменной {}", p.base)),
        };
        match walk(&mut var.slot, &steps, p)? {
            Some(Value::Enum(_, vi, _)) => Ok(Key::Variant(*vi)),
            Some(Value::Int(_, n)) => Ok(Key::Int(*n)),
            Some(Value::Char(c)) => Ok(Key::Char(*c)),
            Some(Value::Bool(b)) => Ok(Key::Bool(*b)),
            Some(_) => invalid(format!(
                "{} — не вариант и не целое, символ или bool",
                ir_text::place(p)
            )),
            None => invalid(format!("в {} нет значения", ir_text::place(p))),
        }
    }

    fn matches(&self, key: &Key, ty: &Type, label: &CaseLabel) -> Result<bool, Stop> {
        match (key, label) {
            (Key::Variant(vi), CaseLabel::Variant(name)) => {
                let e = match ty {
                    Type::Named(n) => self.prog.enums.get(n.as_str()),
                    _ => None,
                };
                match e {
                    Some(e) if e.variants.iter().any(|v| v.name == *name) => {
                        Ok(e.variants[*vi].name == *name)
                    }
                    _ => invalid(format!("case {}: такого варианта нет", name)),
                }
            }
            (Key::Int(n), CaseLabel::Value(Const::Int(m))) => Ok(n == m),
            (Key::Char(c), CaseLabel::Value(Const::Char(d))) => Ok(c == d),
            (Key::Bool(b), CaseLabel::Value(Const::Bool(d))) => Ok(b == d),
            _ => invalid("метка case не подходит к значению switch".to_string()),
        }
    }

    /// Константа получает тип места, где стоит.
    fn constant(&self, c: &Const, expected: &Type) -> Result<Value, Stop> {
        match (c, expected) {
            (Const::Int(n), Type::Scalar(s)) if s.int_range().is_some() => match s.int_range() {
                Some((lo, hi)) if lo <= *n && *n <= hi => Ok(Value::Int(*s, *n)),
                _ => invalid(format!("константа {} не помещается в {}", n, s.name())),
            },
            (Const::Float(bits), Type::Scalar(s @ (Scalar::F32 | Scalar::F64))) => {
                let x = f64::from_bits(*bits);
                if !x.is_finite() {
                    return invalid("дробная константа должна быть конечной".to_string());
                }
                if *s == Scalar::F64 {
                    return Ok(Value::F64(x));
                }
                let y = x as f32;
                if (y as f64).to_bits() == *bits {
                    Ok(Value::F32(y))
                } else {
                    invalid(format!("константа {:?} — не точное значение f32", x))
                }
            }
            (Const::Bool(b), Type::Scalar(Scalar::Bool)) => Ok(Value::Bool(*b)),
            (Const::Char(ch), Type::Scalar(Scalar::Char)) => Ok(Value::Char(*ch)),
            (Const::Unit, Type::Unit) => Ok(Value::Unit),
            _ => invalid(format!(
                "константа не подходит к типу {}",
                ir_text::ty(expected)
            )),
        }
    }

    fn operand(&mut self, frame: &mut Frame, o: &Operand, expected: &Type) -> Result<Value, Stop> {
        let v = match o {
            Operand::Const(c) => self.constant(c, expected)?,
            Operand::Copy(p) => self.copy_of(frame, p)?,
            Operand::Move(p) => self.take_from(frame, p, false)?.0,
            Operand::Borrow(p) | Operand::BorrowMut(p) => {
                return invalid(format!(
                    "заимствование {} бывает только аргументом вызова: ссылок как значений в шаге 2 нет",
                    ir_text::place(p)
                ))
            }
        };
        if !self.prog.conforms(&v, expected) {
            return invalid(format!(
                "значение {} не подходит к типу {}",
                show(&v),
                ir_text::ty(expected)
            ));
        }
        Ok(v)
    }

    /// Величина сдвига: целое любого типа.
    fn shift_count(&mut self, frame: &mut Frame, o: &Operand) -> Result<Value, Stop> {
        let v = match o {
            Operand::Const(Const::Int(n)) => return Ok(Value::Int(Scalar::I64, *n)),
            Operand::Copy(p) => self.copy_of(frame, p)?,
            Operand::Move(p) => self.take_from(frame, p, false)?.0,
            _ => return invalid("величина сдвига — целое".to_string()),
        };
        match v {
            Value::Int(..) => Ok(v),
            _ => invalid(format!("величина сдвига {} — не целое", show(&v))),
        }
    }

    fn rhs(&mut self, frame: &mut Frame, r: &Rhs, expected: &Type) -> Result<Value, Stop> {
        let v = match r {
            Rhs::Use(o) => self.operand(frame, o, expected)?,
            Rhs::Op { op, ty, args } => {
                let mut values = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    let v = if i == 1 && matches!(op, Op::Shl | Op::Shr) {
                        self.shift_count(frame, a)?
                    } else {
                        self.operand(frame, a, &Type::Scalar(*ty))?
                    };
                    values.push(v);
                }
                eval_op(*op, *ty, &values)?
            }
            Rhs::Conv { from, to, arg } => {
                let v = self.operand(frame, arg, &Type::Scalar(*from))?;
                eval_conv(*from, *to, &v)?
            }
            Rhs::Call { func, args } => self.call(frame, func, args)?,
            Rhs::Make { ty, variant, args } => {
                self.make(frame, ty, variant.as_deref(), args.as_deref())?
            }
            Rhs::Tuple(items) => match expected {
                Type::Tuple(ts) if ts.len() == items.len() => {
                    let mut xs = Vec::new();
                    for (o, t) in items.iter().zip(ts) {
                        xs.push(Some(self.operand(frame, o, t)?));
                    }
                    Value::Tuple(xs)
                }
                _ => {
                    return invalid(format!(
                        "кортеж не подходит к типу {}",
                        ir_text::ty(expected)
                    ))
                }
            },
            Rhs::Array(items) => match expected {
                Type::Array(t, n) if *n == items.len() as u64 => {
                    let mut xs = Vec::new();
                    for o in items {
                        xs.push(Some(self.operand(frame, o, t)?));
                    }
                    Value::Array(xs)
                }
                _ => {
                    return invalid(format!(
                        "массив не подходит к типу {}",
                        ir_text::ty(expected)
                    ))
                }
            },
        };
        if !self.prog.conforms(&v, expected) {
            return invalid(format!(
                "значение {} не подходит к типу {}",
                show(&v),
                ir_text::ty(expected)
            ));
        }
        Ok(v)
    }

    /// Поля по именам в порядке записи; каждое ровно один раз.
    fn named_fields(
        &mut self,
        frame: &mut Frame,
        fields: &[Field],
        args: &[Arg],
        what: &str,
    ) -> Result<Vec<Slot>, Stop> {
        let mut slots: Vec<Slot> = vec![None; fields.len()];
        for a in args {
            let k = match &a.name {
                Some(name) => fields.iter().position(|f| f.name == *name),
                None => None,
            };
            let k = match k {
                Some(k) if slots[k].is_none() => k,
                _ => return invalid(format!("{}: поля указаны не так, как объявлены", what)),
            };
            slots[k] = Some(self.operand(frame, &a.value, &fields[k].ty)?);
        }
        if slots.iter().any(Option::is_none) {
            return invalid(format!("{}: указаны не все поля", what));
        }
        Ok(slots)
    }

    fn make(
        &mut self,
        frame: &mut Frame,
        ty: &str,
        variant: Option<&str>,
        args: Option<&[Arg]>,
    ) -> Result<Value, Stop> {
        let what = match variant {
            Some(v) => format!("make {}.{}", ty, v),
            None => format!("make {}", ty),
        };
        let vname = match variant {
            None => {
                let s: &'m StructDecl = match self.prog.structs.get(ty) {
                    Some(s) => s,
                    None => return invalid(format!("{}: нет такой записи", what)),
                };
                let args = match args {
                    Some(args) => args,
                    None => return invalid(format!("{}: поля записи — в скобках", what)),
                };
                let fields = self.named_fields(frame, &s.fields, args, &what)?;
                return Ok(Value::Struct(ty.to_string(), fields));
            }
            Some(v) => v,
        };
        let e: &'m EnumDecl = match self.prog.enums.get(ty) {
            Some(e) => e,
            None => return invalid(format!("{}: нет такого перечисления", what)),
        };
        let vi = match e.variants.iter().position(|v| v.name == vname) {
            Some(vi) => vi,
            None => return invalid(format!("{}: нет такого варианта", what)),
        };
        let payload = match (&e.variants[vi].payload, args) {
            (Payload::None, None) => Vec::new(),
            (Payload::One(t), Some([a])) if a.name.is_none() => {
                vec![Some(self.operand(frame, &a.value, t)?)]
            }
            (Payload::Fields(fs), Some(args)) => self.named_fields(frame, fs, args, &what)?,
            _ => {
                return invalid(format!(
                    "{}: значения не подходят к объявлению варианта",
                    what
                ))
            }
        };
        Ok(Value::Enum(ty.to_string(), vi, payload))
    }

    /// Вызов: аргументы по порядку и по режимам параметров, затем тело, затем возврат
    /// значений параметров `mut` на их места.
    fn call(&mut self, frame: &mut Frame, name: &str, args: &[Operand]) -> Result<Value, Stop> {
        let f = self.func(name)?;
        if args.len() != f.params.len() {
            return invalid(format!(
                "функция {} берёт аргументов: {}; дано: {}",
                f.name,
                f.params.len(),
                args.len()
            ));
        }
        let mut vars = HashMap::new();
        let mut lent: Vec<(&'m str, &Place, Vec<Step>)> = Vec::new();
        for (p, a) in f.params.iter().zip(args) {
            self.prog.check_type(&p.ty)?;
            let v = match (p.mode, a) {
                (Mode::Read, Operand::Borrow(place)) => self.lend_read(frame, place)?,
                (Mode::Read, Operand::Copy(_) | Operand::Const(_)) => {
                    self.operand(frame, a, &p.ty)?
                }
                (Mode::Mut, Operand::BorrowMut(place)) => {
                    let (v, steps) = self.take_from(frame, place, true)?;
                    lent.push((&p.name, place, steps));
                    v
                }
                (Mode::Take, Operand::Move(_) | Operand::Copy(_) | Operand::Const(_)) => {
                    self.operand(frame, a, &p.ty)?
                }
                _ => {
                    return invalid(format!(
                        "вызов {}: аргумент не подходит к режиму параметра {}",
                        f.name, p.name
                    ))
                }
            };
            if !self.prog.conforms(&v, &p.ty) {
                return invalid(format!(
                    "вызов {}: аргумент {} не подходит к параметру {}",
                    f.name,
                    show(&v),
                    p.name
                ));
            }
            let var = Var {
                ty: p.ty.clone(),
                mode: Some(p.mode),
                slot: Some(v),
            };
            if vars.insert(p.name.clone(), var).is_some() {
                return invalid(format!("функция {}: параметр {} повторён", f.name, p.name));
            }
        }
        let (value, mut callee) = self.invoke(f, vars)?;
        for (pname, place, steps) in lent {
            let back = callee.vars.remove(pname).and_then(|var| var.slot);
            let back = match back {
                Some(v) if is_complete(&v) => v,
                _ => {
                    return invalid(format!(
                        "функция {}: параметр mut {} к возврату не целиком",
                        f.name, pname
                    ))
                }
            };
            self.give_back(frame, place, &steps, back)?;
        }
        Ok(value)
    }
}
