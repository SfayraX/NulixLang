//! Интерпретатор IR: смысл инструкций.
//!
//! Целая арифметика сверяется с проверяющими операциями Rust: все пары значений `i8` и `u8`,
//! для типов шире — случайные пары и крайние значения. Преобразования сверяются с независимыми
//! расчётами: целое — через `TryFrom`, дробное в целое — через десятичную запись, целое в
//! дробное — через соседние значения. Остальное — программы на IR, написанные вручную, и
//! пример из `spec/ir-core.md`. Программы лежат в `tests/ir/`: их же проверяет тест проверщика.

use nulixc::interp::{self, eval_conv, eval_op, Limits, Stop, Value};
use nulixc::ir::*;
use nulixc::ir_text::parse;

// ───────── помощники ─────────

fn module(text: &str) -> Module {
    match parse(text) {
        Ok(m) => m,
        Err(e) => panic!("строка {}: {}\n{}", e.line, e.message, text),
    }
}

fn call(m: &Module, func: &str, args: Vec<Value>) -> Result<Value, Stop> {
    interp::call(m, func, args, Limits::default())
}

fn i64v(n: i64) -> Value {
    Value::Int(Scalar::I64, n as i128)
}

fn u64v(n: u64) -> Value {
    Value::Int(Scalar::U64, n as i128)
}

#[track_caller]
fn crash_with(r: Result<Value, Stop>, fragment: &str) {
    match r {
        Err(Stop::Crash(m)) if m.contains(fragment) => {}
        other => panic!("ожидалась авария «{}», а вышло {:?}", fragment, other),
    }
}

#[track_caller]
fn invalid_with(r: Result<Value, Stop>, fragment: &str) {
    match r {
        Err(Stop::InvalidIr(m)) if m.contains(fragment) => {}
        other => panic!("ожидался неверный IR «{}», а вышло {:?}", fragment, other),
    }
}

/// Генератор splitmix64 с постоянным зерном: прогон повторяется в точности.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

// ───────── целые: сверка с Rust ─────────

/// Проверяющие операции Rust для одного целого типа.
trait Int:
    Copy
    + Ord
    + std::fmt::Debug
    + Into<i128>
    + TryFrom<i128>
    + std::ops::BitAnd<Output = Self>
    + std::ops::BitOr<Output = Self>
    + std::ops::BitXor<Output = Self>
    + std::ops::Not<Output = Self>
{
    const S: Scalar;
    const SIGNED: bool;
    fn add(self, b: Self) -> Option<Self>;
    fn sub(self, b: Self) -> Option<Self>;
    fn mul(self, b: Self) -> Option<Self>;
    fn div(self, b: Self) -> Option<Self>;
    fn rem(self, b: Self) -> Option<Self>;
    fn neg(self) -> Option<Self>;
    fn shl(self, n: u32) -> Option<Self>;
    fn shr(self, n: u32) -> Option<Self>;
}

impl Int for i8 {
    const S: Scalar = Scalar::I8;
    const SIGNED: bool = true;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        self.checked_neg()
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for i16 {
    const S: Scalar = Scalar::I16;
    const SIGNED: bool = true;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        self.checked_neg()
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for i32 {
    const S: Scalar = Scalar::I32;
    const SIGNED: bool = true;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        self.checked_neg()
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for i64 {
    const S: Scalar = Scalar::I64;
    const SIGNED: bool = true;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        self.checked_neg()
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for u8 {
    const S: Scalar = Scalar::U8;
    const SIGNED: bool = false;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        None
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for u16 {
    const S: Scalar = Scalar::U16;
    const SIGNED: bool = false;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        None
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for u32 {
    const S: Scalar = Scalar::U32;
    const SIGNED: bool = false;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        None
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

impl Int for u64 {
    const S: Scalar = Scalar::U64;
    const SIGNED: bool = false;
    fn add(self, b: Self) -> Option<Self> {
        self.checked_add(b)
    }
    fn sub(self, b: Self) -> Option<Self> {
        self.checked_sub(b)
    }
    fn mul(self, b: Self) -> Option<Self> {
        self.checked_mul(b)
    }
    fn div(self, b: Self) -> Option<Self> {
        self.checked_div(b)
    }
    fn rem(self, b: Self) -> Option<Self> {
        self.checked_rem(b)
    }
    fn neg(self) -> Option<Self> {
        None
    }
    fn shl(self, n: u32) -> Option<Self> {
        self.checked_shl(n)
    }
    fn shr(self, n: u32) -> Option<Self> {
        self.checked_shr(n)
    }
}

fn val<T: Int>(x: T) -> Value {
    Value::Int(T::S, x.into())
}

/// Ожидание сходится с итогом интерпретатора: значение или авария.
#[track_caller]
fn same(expected: Option<Value>, got: Result<Value, Stop>, what: &str) {
    match (&expected, &got) {
        (Some(e), Ok(g)) if e == g => {}
        (None, Err(Stop::Crash(_))) => {}
        _ => panic!("{}: ожидалось {:?}, вышло {:?}", what, expected, got),
    }
}

const BINARY: [Op; 14] = [
    Op::Add,
    Op::Sub,
    Op::Mul,
    Op::Div,
    Op::Rem,
    Op::And,
    Op::Or,
    Op::Xor,
    Op::Eq,
    Op::Ne,
    Op::Lt,
    Op::Le,
    Op::Gt,
    Op::Ge,
];

fn check_pair<T: Int>(a: T, b: T) {
    for op in BINARY {
        let expected = match op {
            Op::Add => a.add(b).map(val),
            Op::Sub => a.sub(b).map(val),
            Op::Mul => a.mul(b).map(val),
            Op::Div => a.div(b).map(val),
            // MIN % -1 по правилу Nulix — 0, а в Rust это переполнение
            Op::Rem => match (a.rem(b), a.div(b)) {
                (None, None) if b.into() == -1 => Some(Value::Int(T::S, 0)),
                (r, _) => r.map(val),
            },
            Op::And => Some(val(a & b)),
            Op::Or => Some(val(a | b)),
            Op::Xor => Some(val(a ^ b)),
            Op::Eq => Some(Value::Bool(a == b)),
            Op::Ne => Some(Value::Bool(a != b)),
            Op::Lt => Some(Value::Bool(a < b)),
            Op::Le => Some(Value::Bool(a <= b)),
            Op::Gt => Some(Value::Bool(a > b)),
            _ => Some(Value::Bool(a >= b)),
        };
        let got = eval_op(op, T::S, &[val(a), val(b)]);
        same(
            expected,
            got,
            &format!("{}.{}({:?}, {:?})", op.name(), T::S.name(), a, b),
        );
    }
}

fn check_unary_and_shifts<T: Int>(a: T, counts: &[i128]) {
    let neg = eval_op(Op::Neg, T::S, &[val(a)]);
    if T::SIGNED {
        same(a.neg().map(val), neg, &format!("neg {:?}", a));
    } else {
        invalid_with(neg, "нет такой операции");
    }
    same(
        Some(val(!a)),
        eval_op(Op::Not, T::S, &[val(a)]),
        &format!("not {:?}", a),
    );
    for &c in counts {
        for (op, f) in [
            (Op::Shl, T::shl as fn(T, u32) -> Option<T>),
            (Op::Shr, T::shr),
        ] {
            let expected = u32::try_from(c).ok().and_then(|n| f(a, n)).map(val);
            // величина сдвига — целое любого типа: здесь i64 и u8
            let got = eval_op(op, T::S, &[val(a), Value::Int(Scalar::I64, c)]);
            same(
                expected.clone(),
                got,
                &format!("{} {:?} на {}", op.name(), a, c),
            );
            if (0..=255).contains(&c) {
                let got = eval_op(op, T::S, &[val(a), Value::Int(Scalar::U8, c)]);
                same(expected, got, &format!("{} {:?} на u8 {}", op.name(), a, c));
            }
        }
    }
}

fn counts() -> Vec<i128> {
    let mut c: Vec<i128> = (-3..=67).collect();
    c.extend([1000, i64::MIN as i128, u64::MAX as i128]);
    c
}

#[test]
fn all_pairs_of_8_bit_integers_match_rust() {
    let counts = counts();
    for a in i8::MIN..=i8::MAX {
        for b in i8::MIN..=i8::MAX {
            check_pair(a, b);
        }
        check_unary_and_shifts(a, &counts);
    }
    for a in u8::MIN..=u8::MAX {
        for b in u8::MIN..=u8::MAX {
            check_pair(a, b);
        }
        check_unary_and_shifts(a, &counts);
    }
}

/// Крайние и случайные значения типа.
fn samples<T: Int>(r: &mut Rng, n: usize) -> Vec<T> {
    let edges: [i128; 9] = [
        0,
        1,
        -1,
        2,
        i64::MIN as i128,
        i64::MAX as i128,
        u64::MAX as i128,
        i32::MIN as i128,
        u32::MAX as i128,
    ];
    let mut out: Vec<T> = edges.iter().filter_map(|&e| T::try_from(e).ok()).collect();
    let bits = std::mem::size_of::<T>() as u32 * 8;
    for _ in 0..n {
        let x = r.next();
        // треть малых чисел, остальное — во всю ширину
        let v: i128 = if x.is_multiple_of(3) {
            (r.next() % 200) as i128 - 100
        } else if T::SIGNED {
            ((x << (64 - bits)) as i64 >> (64 - bits)) as i128
        } else {
            (x >> (64 - bits)) as i128
        };
        if let Ok(t) = T::try_from(v) {
            out.push(t);
        }
    }
    // MIN и MAX самого типа
    let lo = if T::SIGNED { -(1i128 << (bits - 1)) } else { 0 };
    let hi = if T::SIGNED {
        (1i128 << (bits - 1)) - 1
    } else {
        (1i128 << bits) - 1
    };
    for e in [lo, lo + 1, hi, hi - 1] {
        if let Ok(t) = T::try_from(e) {
            out.push(t);
        }
    }
    out
}

fn check_wide<T: Int>(seed: u64) {
    let mut r = Rng(seed);
    let xs: Vec<T> = samples(&mut r, 300);
    let counts = counts();
    for &a in &xs {
        for &b in &xs {
            check_pair(a, b);
        }
        check_unary_and_shifts(a, &counts);
    }
}

#[test]
fn wider_integers_match_rust() {
    check_wide::<i16>(1);
    check_wide::<u16>(2);
    check_wide::<i32>(3);
    check_wide::<u32>(4);
    check_wide::<i64>(5);
    check_wide::<u64>(6);
}

#[test]
fn integer_rules_named_in_the_spec() {
    let i = |n: i64| Value::Int(Scalar::I64, n as i128);
    // деление к нулю, остаток со знаком делимого
    assert_eq!(eval_op(Op::Div, Scalar::I64, &[i(-7), i(2)]), Ok(i(-3)));
    assert_eq!(eval_op(Op::Rem, Scalar::I64, &[i(-7), i(2)]), Ok(i(-1)));
    assert_eq!(eval_op(Op::Rem, Scalar::I64, &[i(7), i(-2)]), Ok(i(1)));
    assert_eq!(
        eval_op(Op::Rem, Scalar::I64, &[i(i64::MIN), i(-1)]),
        Ok(i(0))
    );
    crash_with(
        eval_op(Op::Div, Scalar::I64, &[i(i64::MIN), i(-1)]),
        "переполнение",
    );
    crash_with(
        eval_op(Op::Div, Scalar::I64, &[i(1), i(0)]),
        "деление на ноль",
    );
    crash_with(
        eval_op(Op::Rem, Scalar::I64, &[i(1), i(0)]),
        "деление на ноль",
    );
    // сдвиг ровно на ширину — авария; выдвинутые биты не проверяются; >> у знаковых арифметический
    crash_with(
        eval_op(Op::Shl, Scalar::U32, &[Value::Int(Scalar::U32, 1), i(32)]),
        "сдвиг на 32 при ширине 32",
    );
    crash_with(eval_op(Op::Shr, Scalar::I64, &[i(1), i(-1)]), "сдвиг на -1");
    assert_eq!(
        eval_op(
            Op::Shl,
            Scalar::U8,
            &[Value::Int(Scalar::U8, 0b1100_0001), i(2)]
        ),
        Ok(Value::Int(Scalar::U8, 0b0000_0100))
    );
    assert_eq!(eval_op(Op::Shr, Scalar::I64, &[i(-8), i(1)]), Ok(i(-4)));
    assert_eq!(
        eval_op(Op::Shr, Scalar::U8, &[Value::Int(Scalar::U8, 0x80), i(7)]),
        Ok(Value::Int(Scalar::U8, 1))
    );
}

// ───────── преобразования ─────────

const INTS: [Scalar; 8] = [
    Scalar::I8,
    Scalar::I16,
    Scalar::I32,
    Scalar::I64,
    Scalar::U8,
    Scalar::U16,
    Scalar::U32,
    Scalar::U64,
];

/// Помещается ли целое в тип — по `TryFrom` из Rust.
fn fits(to: Scalar, n: i128) -> bool {
    match to {
        Scalar::I8 => i8::try_from(n).is_ok(),
        Scalar::I16 => i16::try_from(n).is_ok(),
        Scalar::I32 => i32::try_from(n).is_ok(),
        Scalar::I64 => i64::try_from(n).is_ok(),
        Scalar::U8 => u8::try_from(n).is_ok(),
        Scalar::U16 => u16::try_from(n).is_ok(),
        Scalar::U32 => u32::try_from(n).is_ok(),
        Scalar::U64 => u64::try_from(n).is_ok(),
        _ => panic!("не целое"),
    }
}

fn int_values(r: &mut Rng) -> Vec<i128> {
    let mut v: Vec<i128> = Vec::new();
    for s in [
        i8::MIN as i128,
        i8::MAX as i128,
        u8::MAX as i128,
        i16::MIN as i128,
        i16::MAX as i128,
        u16::MAX as i128,
        i32::MIN as i128,
        i32::MAX as i128,
        u32::MAX as i128,
        i64::MIN as i128,
        i64::MAX as i128,
        u64::MAX as i128,
    ] {
        v.extend([s - 1, s, s + 1]);
    }
    v.extend([-1, 0, 1]);
    for _ in 0..200 {
        v.push(r.next() as i64 as i128);
        v.push(r.next() as i128);
        v.push((r.next() % 1000) as i128 - 500);
    }
    v
}

#[test]
fn integer_to_integer_checks_the_range() {
    let mut r = Rng(7);
    let values = int_values(&mut r);
    for from in INTS {
        for &n in values.iter().filter(|&&n| fits(from, n)) {
            for to in INTS {
                let expected = fits(to, n).then_some(Value::Int(to, n));
                let got = eval_conv(from, to, &Value::Int(from, n));
                same(
                    expected,
                    got,
                    &format!("conv.{}.{}({})", from.name(), to.name(), n),
                );
            }
        }
    }
}

/// Дробное в целое по десятичной записи отброшенной дробной части — независимо от сравнений
/// дробных, которыми пользуется интерпретатор.
fn trunc_oracle(x: f64, to: Scalar) -> Option<i128> {
    if !x.is_finite() {
        return None;
    }
    let n: i128 = format!("{:.0}", x.trunc()).parse().ok()?;
    fits(to, n).then_some(n)
}

fn float_values(r: &mut Rng) -> Vec<f64> {
    let mut v = vec![
        0.0,
        -0.0,
        0.5,
        -0.5,
        0.9999999999999999,
        -0.9999999999999999,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        1e300,
        -1e300,
    ];
    for b in [7i32, 8, 15, 16, 31, 32, 63, 64] {
        let p = 2f64.powi(b);
        for x in [p, -p, p - 1.0, -p + 1.0, p + 1.0, -p - 1.0] {
            v.extend([x, x.next_up(), x.next_down(), x + 0.5, x - 0.5]);
        }
    }
    for _ in 0..2000 {
        let bits = r.next();
        v.push(f64::from_bits(bits));
        // значения в пределах 64-битных целых
        v.push((r.next() as i64 as f64) * 0.75);
    }
    v
}

#[test]
fn float_to_integer_truncates_and_checks_the_range() {
    let mut r = Rng(8);
    for x in float_values(&mut r) {
        for to in INTS {
            let expected = trunc_oracle(x, to).map(|n| Value::Int(to, n));
            same(
                expected.clone(),
                eval_conv(Scalar::F64, to, &Value::F64(x)),
                &format!("conv.f64.{}({:?})", to.name(), x),
            );
            let y = x as f32;
            let expected = trunc_oracle(y as f64, to).map(|n| Value::Int(to, n));
            same(
                expected,
                eval_conv(Scalar::F32, to, &Value::F32(y)),
                &format!("conv.f32.{}({:?})", to.name(), y),
            );
        }
    }
}

/// `r` — ближайшее к `n` из чисел вида f64, при равенстве — с чётной мантиссой.
fn nearest_f64(n: i128, r: f64) -> bool {
    let ri = r as i128;
    if r != ri as f64 || ri == n {
        return ri == n;
    }
    let (up, down) = (r.next_up() as i128, r.next_down() as i128);
    let (d, du, dd) = ((n - ri).abs(), (n - up).abs(), (n - down).abs());
    d <= du && d <= dd && (d < du && d < dd || r.to_bits() & 1 == 0)
}

fn nearest_f32(n: i128, r: f32) -> bool {
    let ri = r as i128;
    if r != ri as f32 || ri == n {
        return ri == n;
    }
    let (up, down) = (r.next_up() as i128, r.next_down() as i128);
    let (d, du, dd) = ((n - ri).abs(), (n - up).abs(), (n - down).abs());
    d <= du && d <= dd && (d < du && d < dd || r.to_bits() & 1 == 0)
}

#[test]
fn integer_to_float_rounds_to_nearest_even() {
    let mut r = Rng(9);
    let mut values = int_values(&mut r);
    for p in [24, 25, 53, 54, 63] {
        let x = 1i128 << p;
        values.extend([x - 1, x + 1, x + 3, -x - 1, -x - 3]);
    }
    // ловушка двойного округления: через f64 вышло бы 2^60, а ближайшее f32 — 2^60 + 2^37
    let trap = (1i128 << 60) + (1i128 << 36) + 1;
    values.extend([trap, -trap]);
    for from in INTS {
        for &n in values.iter().filter(|&&n| fits(from, n)) {
            match eval_conv(from, Scalar::F64, &Value::Int(from, n)) {
                Ok(Value::F64(x)) => assert!(nearest_f64(n, x), "{} → {:?}", n, x),
                other => panic!("{} в f64: {:?}", n, other),
            }
            match eval_conv(from, Scalar::F32, &Value::Int(from, n)) {
                Ok(Value::F32(x)) => assert!(nearest_f32(n, x), "{} → {:?}", n, x),
                other => panic!("{} в f32: {:?}", n, other),
            }
        }
    }
}

#[test]
fn f64_to_f32_crashes_only_on_finite_overflow() {
    let max = f32::MAX as f64;
    // ровно посередине между f32::MAX и 2^128 округление к чётному даёт бесконечность
    let tie = max + 2f64.powi(103);
    let cases: [(f64, Option<f32>); 10] = [
        (0.1, Some(0.1f32)),
        (max, Some(f32::MAX)),
        (tie.next_down(), Some(f32::MAX)),
        (tie, None),
        (-tie, None),
        (1e39, None),
        (1e-50, Some(0.0)),
        (f64::INFINITY, Some(f32::INFINITY)),
        (f64::NEG_INFINITY, Some(f32::NEG_INFINITY)),
        (-0.0, Some(-0.0f32)),
    ];
    for (x, expected) in cases {
        let got = eval_conv(Scalar::F64, Scalar::F32, &Value::F64(x));
        match (expected, &got) {
            (Some(e), Ok(Value::F32(g))) if e.to_bits() == g.to_bits() => {}
            (None, Err(Stop::Crash(m))) if m.contains("не помещается") => {}
            _ => panic!(
                "conv.f64.f32({:?}): ожидалось {:?}, вышло {:?}",
                x, expected, got
            ),
        }
    }
    match eval_conv(Scalar::F64, Scalar::F32, &Value::F64(f64::NAN)) {
        Ok(Value::F32(x)) if x.is_nan() => {}
        other => panic!("NaN в f32: {:?}", other),
    }
    assert_eq!(
        eval_conv(Scalar::F32, Scalar::F64, &Value::F32(0.1)),
        Ok(Value::F64(0.1f32 as f64))
    );
}

#[test]
fn characters_and_booleans_convert_as_decided() {
    let c = |n: i128| eval_conv(Scalar::U32, Scalar::Char, &Value::Int(Scalar::U32, n));
    assert_eq!(c(0x41), Ok(Value::Char('A')));
    assert_eq!(c(0x10FFFF), Ok(Value::Char('\u{10FFFF}')));
    crash_with(c(0xD800), "не символ Unicode");
    crash_with(c(0xDFFF), "не символ Unicode");
    crash_with(c(0x110000), "не символ Unicode");
    crash_with(
        eval_conv(Scalar::I64, Scalar::Char, &Value::Int(Scalar::I64, -1)),
        "не символ Unicode",
    );
    assert_eq!(
        eval_conv(Scalar::Char, Scalar::U8, &Value::Char('ÿ')),
        Ok(Value::Int(Scalar::U8, 255))
    );
    crash_with(
        eval_conv(Scalar::Char, Scalar::U8, &Value::Char('Ā')),
        "не помещается",
    );
    crash_with(
        eval_conv(Scalar::Char, Scalar::I16, &Value::Char('😀')),
        "не помещается",
    );
    assert_eq!(
        eval_conv(Scalar::Char, Scalar::U32, &Value::Char('😀')),
        Ok(Value::Int(Scalar::U32, 0x1F600))
    );
    invalid_with(
        eval_conv(Scalar::Bool, Scalar::U8, &Value::Bool(true)),
        "bool ни во что",
    );
    invalid_with(
        eval_conv(Scalar::U8, Scalar::Bool, &Value::Int(Scalar::U8, 1)),
        "bool ни во что",
    );
    invalid_with(
        eval_conv(Scalar::Char, Scalar::F64, &Value::Char('a')),
        "друг в друга",
    );
    invalid_with(
        eval_conv(Scalar::F64, Scalar::Char, &Value::F64(65.0)),
        "друг в друга",
    );
    // операнд не того типа, что указан в conv
    invalid_with(
        eval_conv(Scalar::I32, Scalar::I64, &Value::Int(Scalar::I64, 1)),
        "операнд не типа i32",
    );
}

// ───────── дробные ─────────

#[test]
fn float_arithmetic_follows_ieee_without_crashes() {
    let f = Value::F64;
    let op = |o: Op, a: f64, b: f64| eval_op(o, Scalar::F64, &[f(a), f(b)]);
    let is_nan = |r: Result<Value, Stop>| matches!(r, Ok(Value::F64(x)) if x.is_nan());
    assert_eq!(op(Op::Div, 1.0, 0.0), Ok(f(f64::INFINITY)));
    assert_eq!(op(Op::Div, -1.0, 0.0), Ok(f(f64::NEG_INFINITY)));
    assert!(is_nan(op(Op::Div, 0.0, 0.0)));
    assert_eq!(op(Op::Mul, f64::MAX, 2.0), Ok(f(f64::INFINITY)));
    assert!(is_nan(op(Op::Add, f64::INFINITY, f64::NEG_INFINITY)));
    // остаток как fmod: знак делимого, при нулевом делителе NaN
    assert_eq!(op(Op::Rem, -5.5, 2.0), Ok(f(-1.5)));
    assert_eq!(op(Op::Rem, 5.5, -2.0), Ok(f(1.5)));
    assert!(is_nan(op(Op::Rem, 5.0, 0.0)));
    assert!(is_nan(op(Op::Rem, f64::INFINITY, 3.0)));
    assert_eq!(op(Op::Rem, 3.0, f64::INFINITY), Ok(f(3.0)));
    match op(Op::Rem, -0.0, 1.0) {
        Ok(Value::F64(x)) => assert_eq!(x.to_bits(), (-0.0f64).to_bits()),
        other => panic!("{:?}", other),
    }
    // сравнения по IEEE 754
    let nan = f64::NAN;
    assert_eq!(op(Op::Eq, nan, nan), Ok(Value::Bool(false)));
    assert_eq!(op(Op::Ne, nan, nan), Ok(Value::Bool(true)));
    assert_eq!(op(Op::Lt, nan, 1.0), Ok(Value::Bool(false)));
    assert_eq!(op(Op::Ge, nan, nan), Ok(Value::Bool(false)));
    assert_eq!(op(Op::Eq, -0.0, 0.0), Ok(Value::Bool(true)));
    match eval_op(Op::Neg, Scalar::F64, &[f(0.0)]) {
        Ok(Value::F64(x)) => assert_eq!(x.to_bits(), (-0.0f64).to_bits()),
        other => panic!("{:?}", other),
    }
    // f32 считается в одинарной точности, а не в двойной
    assert_eq!(
        eval_op(
            Op::Add,
            Scalar::F32,
            &[Value::F32(16777216.0), Value::F32(1.0)]
        ),
        Ok(Value::F32(16777216.0))
    );
    invalid_with(
        eval_op(Op::Shl, Scalar::F64, &[f(1.0), f(1.0)]),
        "нет такой операции",
    );
}

#[test]
fn operations_exist_only_where_decided() {
    let b = Value::Bool;
    assert_eq!(
        eval_op(Op::Xor, Scalar::Bool, &[b(true), b(true)]),
        Ok(b(false))
    );
    assert_eq!(eval_op(Op::Not, Scalar::Bool, &[b(false)]), Ok(b(true)));
    assert_eq!(
        eval_op(Op::Ne, Scalar::Bool, &[b(true), b(false)]),
        Ok(b(true))
    );
    invalid_with(
        eval_op(Op::Lt, Scalar::Bool, &[b(false), b(true)]),
        "нет такой операции",
    );
    let c = Value::Char;
    assert_eq!(
        eval_op(Op::Lt, Scalar::Char, &[c('a'), c('я')]),
        Ok(b(true))
    );
    invalid_with(
        eval_op(Op::Add, Scalar::Char, &[c('a'), c('b')]),
        "нет такой операции",
    );
    invalid_with(
        eval_op(Op::Neg, Scalar::U8, &[Value::Int(Scalar::U8, 1)]),
        "нет такой операции",
    );
    invalid_with(
        eval_op(Op::Add, Scalar::I64, &[i64v(1), Value::Int(Scalar::I32, 1)]),
        "операнды не того типа",
    );
    invalid_with(
        eval_op(Op::Add, Scalar::I64, &[i64v(1)]),
        "берёт операндов: 2",
    );
}

// ───────── программы ─────────

/// Первый пример IR из `spec/ir-core.md` — функция gcd — исполняется так, как описан.
#[test]
fn the_gcd_example_from_the_spec_runs() {
    let path = format!("{}/../spec/ir-core.md", env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(&path).expect("не найден spec/ir-core.md");
    let start = doc.find("```ir\n").expect("в документе нет примера IR") + "```ir\n".len();
    let end = start + doc[start..].find("```").expect("пример не закрыт");
    let m = module(&doc[start..end]);
    for (a, b, g) in [
        (12u64, 18u64, 6u64),
        (18, 12, 6),
        (0, 5, 5),
        (7, 0, 7),
        (u64::MAX, u64::MAX - 1, 1),
        (1 << 63, 1 << 62, 1 << 62),
    ] {
        assert_eq!(
            call(&m, "gcd", vec![u64v(a), u64v(b)]),
            Ok(u64v(g)),
            "gcd({}, {})",
            a,
            b
        );
    }
}

const FLOW: &str = include_str!("ir/flow.ir");

#[test]
fn branches_and_loops() {
    let m = module(FLOW);
    assert_eq!(call(&m, "count_even", vec![i64v(10)]), Ok(i64v(5)));
    assert_eq!(call(&m, "count_even", vec![i64v(0)]), Ok(i64v(0)));
    assert_eq!(
        call(&m, "first_product", vec![i64v(12), i64v(10)]),
        Ok(i64v(206))
    );
    assert_eq!(
        call(&m, "first_product", vec![i64v(7), i64v(3)]),
        Ok(i64v(-1))
    );
    assert_eq!(call(&m, "classify", vec![Value::Char('a')]), Ok(i64v(1)));
    assert_eq!(call(&m, "classify", vec![Value::Char('я')]), Ok(i64v(2)));
    assert_eq!(call(&m, "classify", vec![Value::Char('b')]), Ok(i64v(0)));
    let i32v = |n: i32| Value::Int(Scalar::I32, n as i128);
    assert_eq!(call(&m, "sign", vec![i32v(0)]), Ok(i32v(0)));
    assert_eq!(call(&m, "sign", vec![i32v(-5)]), Ok(i32v(-1)));
    assert_eq!(call(&m, "sign", vec![i32v(i32::MAX)]), Ok(i32v(1)));
}

const DATA: &str = include_str!("ir/data.ir");

#[test]
fn records_variants_arrays_and_parameter_modes() {
    let m = module(DATA);
    assert_eq!(call(&m, "areas", vec![]), Ok(Value::F64(5.5)));
    assert_eq!(call(&m, "use_swap", vec![]), Ok(i64v(2)));
    assert_eq!(call(&m, "use_take", vec![]), Ok(i64v(3)));
    assert_eq!(call(&m, "use_refill", vec![]), Ok(i64v(74)));
    assert_eq!(call(&m, "use_bump", vec![]), Ok(i64v(11)));
    let point = |x: f64, y: f64| {
        Value::Struct(
            "Point".to_string(),
            vec![Some(Value::F64(x)), Some(Value::F64(y))],
        )
    };
    assert_eq!(
        call(&m, "mirror", vec![point(1.5, 2.0)]),
        Ok(point(-1.5, 2.0))
    );
    assert_eq!(call(&m, "fill_sum", vec![]), Ok(i64v(1020)));
    assert_eq!(call(&m, "grid", vec![]), Ok(i64v(33)));
    assert_eq!(call(&m, "tuples", vec![]), Ok(Value::Char('x')));
    assert_eq!(call(&m, "carry", vec![i64v(4)]), Ok(i64v(6)));
    assert_eq!(call(&m, "carry", vec![i64v(0)]), Ok(i64v(0)));
    assert_eq!(call(&m, "split", vec![Value::Bool(true)]), Ok(i64v(2)));
    assert_eq!(call(&m, "split", vec![Value::Bool(false)]), Ok(i64v(2)));
    assert_eq!(call(&m, "use_unwrap", vec![i64v(5)]), Ok(i64v(-5)));
    let rect = Value::Enum(
        "Shape".to_string(),
        1,
        vec![Some(Value::F64(2.5)), Some(Value::F64(1.0))],
    );
    let dot = Value::Enum("Shape".to_string(), 2, vec![]);
    assert_eq!(
        call(&m, "sum_width", vec![rect, i64v(3)]),
        Ok(Value::F64(7.5))
    );
    assert_eq!(
        call(&m, "sum_width", vec![dot, i64v(3)]),
        Ok(Value::F64(0.0))
    );
    assert_eq!(call(&m, "swap_ends", vec![]), Ok(i64v(31)));
    let left = |n: i64| Value::Enum("Dir".to_string(), 0, vec![Some(i64v(n))]);
    let right = Value::Enum("Dir".to_string(), 1, vec![Some(i64v(1))]);
    assert_eq!(
        call(&m, "turn", vec![left(5), Value::Bool(false)]),
        Ok(i64v(5))
    );
    assert_eq!(
        call(&m, "turn", vec![left(5), Value::Bool(true)]),
        Ok(i64v(-1))
    );
    assert_eq!(
        call(&m, "turn", vec![right.clone(), Value::Bool(false)]),
        Ok(i64v(0))
    );
    assert_eq!(call(&m, "turn_by", vec![right, i64v(0)]), Ok(i64v(0)));
    assert_eq!(call(&m, "use_rebuild", vec![]), Ok(i64v(21)));
    assert_eq!(call(&m, "turn_by", vec![left(5), i64v(0)]), Ok(i64v(-1)));
    assert_eq!(call(&m, "turn_by", vec![left(5), i64v(1)]), Ok(i64v(5)));
    assert_eq!(call(&m, "turn_by", vec![left(5), i64v(2)]), Ok(i64v(-2)));
    assert_eq!(call(&m, "turn_by", vec![left(5), i64v(3)]), Ok(i64v(5)));
    let xs = Value::Array((1..=4).map(|n| Some(i64v(n))).collect());
    assert_eq!(call(&m, "at", vec![xs.clone(), i64v(3)]), Ok(i64v(4)));
    crash_with(
        call(&m, "at", vec![xs.clone(), i64v(4)]),
        "выход за границы: индекс 4, длина 4",
    );
    crash_with(call(&m, "at", vec![xs, i64v(-1)]), "индекс -1");
}

const ORDER: &str = include_str!("ir/order.ir");

#[test]
fn evaluation_order() {
    let m = module(ORDER);
    crash_with(call(&m, "rhs_first", vec![]), "правая часть");
    crash_with(
        call(&m, "bounds_at_write", vec![]),
        "выход за границы: индекс 5, длина 2",
    );
    assert_eq!(call(&m, "index_before_rhs", vec![]), Ok(i64v(109)));
    assert_eq!(call(&m, "left_to_right_ok", vec![]), Ok(Value::Unit));
}

const RUNNER: &str = include_str!("ir/runner.ir");

/// Тест, который вызывает неверную функцию: останавливается только он. В `runner.ir` его нет:
/// этот файл проверщик принимает целиком.
const RUNNER_BAD: &str = "
func lie() -> Never {
  return
}

test \"never возвращается\" {
  call lie()
}
";

#[test]
fn tests_run_one_by_one_within_limits() {
    let m = module(&format!("{}{}", RUNNER, RUNNER_BAD));
    let limits = Limits {
        steps: 10_000,
        depth: 50,
    };
    let results = interp::run_tests(&m, limits).expect("модуль исполняется");
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "проходит",
            "авария",
            "бюджет",
            "стек",
            "never",
            "три шага",
            "после",
            "never возвращается"
        ]
    );
    assert_eq!(results[0].stop, None);
    assert_eq!(results[1].stop, Some(Stop::Crash("бах".to_string())));
    assert_eq!(results[2].stop, Some(Stop::OutOfSteps));
    assert_eq!(
        results[3].stop,
        Some(Stop::Crash(
            "переполнение стека: глубина вызовов больше 50".to_string()
        ))
    );
    assert_eq!(results[4].stop, Some(Stop::Crash("отказ".to_string())));
    assert_eq!(results[5].stop, None);
    assert_eq!(results[6].stop, None);
    assert!(
        matches!(&results[7].stop, Some(Stop::InvalidIr(m)) if m.contains("return без значения"))
    );
    // три инструкции — три шага
    let exact = |steps| {
        let r = interp::run_tests(&m, Limits { steps, depth: 50 }).expect("модуль исполняется");
        r[5].stop.clone()
    };
    assert_eq!(exact(3), None);
    assert_eq!(exact(2), Some(Stop::OutOfSteps));
    // глубина по умолчанию — 1000 вызовов
    assert_eq!(call(&m, "depth", vec![i64v(999)]), Ok(i64v(0)));
    crash_with(
        call(&m, "depth", vec![i64v(1000)]),
        "переполнение стека: глубина вызовов больше 1000",
    );
}

const INVALID: &str = include_str!("ir/invalid.ir");

/// Вызовы функций с параметром `mut`: снаружи их не вызвать. В `invalid.ir` таких вызовов нет:
/// там каждая функция — отдельный случай для проверщика или помощник.
const INVALID_CALLERS: &str = "
func call_move_mut_part() {
  p: Pair = call make_pair()
  call move_mut_part(borrow mut p)
  drop p
}
";

#[test]
fn invalid_ir_stops_with_a_reason() {
    let m = module(&format!("{}{}", INVALID, INVALID_CALLERS));
    let token = Value::Struct("Token".to_string(), vec![Some(i64v(1))]);
    let circle = Value::Enum("Shape".to_string(), 0, vec![Some(Value::F64(1.0))]);
    let xs2 = Value::Array(vec![Some(i64v(1)), Some(i64v(2))]);
    let cases: Vec<(&str, Vec<Value>, &str)> = vec![
        ("read_empty", vec![], "нет значения"),
        ("copy_owned", vec![], "не простой"),
        ("overwrite", vec![], "сначала drop"),
        ("leak", vec![], "не отдано и не уничтожено"),
        ("half_leak", vec![], "значение в p не отдано"),
        ("no_return", vec![], "конец тела достигнут"),
        ("bare_return", vec![], "return без значения"),
        ("no_case", vec![i64v(2)], "нет ветки"),
        ("bad_label", vec![], "нет цикла Nowhere"),
        ("big_const", vec![], "не помещается в u8"),
        ("f32_inexact", vec![], "не точное значение f32"),
        ("borrow_value", vec![], "только аргументом вызова"),
        ("unknown", vec![], "нет функции nobody"),
        ("arity", vec![], "берёт аргументов: 1; дано: 0"),
        ("wrong_mode", vec![], "к режиму параметра"),
        ("drop_param", vec![token.clone()], "не принадлежит"),
        ("move_param", vec![token], "не принадлежит"),
        ("write_param", vec![i64v(1)], "параметр для чтения"),
        ("wrong_variant", vec![circle], "другой вариант"),
        ("discard", vec![], "не используется"),
        ("redeclare", vec![], "объявлена с типом i64"),
        ("lie", vec![], "return без значения"),
        ("left_to_right_bad", vec![], "нет значения"),
        ("read_mut", vec![i64v(1)], "параметр для чтения"),
        ("stale_after_call", vec![], "другой вариант"),
        ("stale_in_loop", vec![], "другой вариант"),
        ("stale_index", vec![], "другой вариант"),
        ("stale_element", vec![], "другой вариант"),
        ("stale_target", vec![], "другой вариант"),
        ("lent_index", vec![], "не переменная i64 со значением"),
        (
            "stale_after_branch",
            vec![Value::Bool(true)],
            "другой вариант",
        ),
        ("stale_after_inner_switch", vec![i64v(0)], "другой вариант"),
        ("stale_in_outer_loop", vec![], "другой вариант"),
        ("stale_after_define", vec![], "другой вариант"),
        (
            "index_empty",
            vec![xs2.clone()],
            "не переменная i64 со значением",
        ),
        ("borrow_after_lend", vec![], "нет значения"),
        ("borrow_empty", vec![], "нет значения"),
        ("lend_empty", vec![], "нет значения"),
        ("move_while_lent", vec![], "часть значения перемещена"),
        ("drop_twice", vec![], "drop t: значения нет"),
        ("if_empty", vec![], "нет значения"),
        (
            "return_leak",
            vec![],
            "значение в t не отдано и не уничтожено",
        ),
        ("write_empty_array", vec![], "на пути к xs[0] нет значения"),
        ("write_into_empty", vec![], "на пути к p.a нет значения"),
        (
            "call_move_mut_part",
            vec![],
            "параметр p функции не принадлежит",
        ),
        ("switch_empty", vec![], "нет значения"),
        ("if_not_bool", vec![i64v(1)], "не bool"),
        (
            "return_wrong_type",
            vec![],
            "константа не подходит к типу i64",
        ),
        ("call_result_type", vec![], "не подходит к типу bool"),
        ("make_wrong_type", vec![], "не подходит к типу Pair"),
        ("make_missing_field", vec![], "указаны не все поля"),
        (
            "array_length",
            vec![],
            "массив не подходит к типу Array[i64, 3]",
        ),
        ("conv_operand_type", vec![i64v(1)], "не подходит к типу i32"),
        ("stale_lent_in_loop", vec![], "другой вариант"),
        ("move_twice", vec![], "в t нет значения"),
        (
            "redeclare_owned",
            vec![],
            "в t ещё есть значение: сначала drop",
        ),
        ("stale_lent_in_define", vec![], "другой вариант"),
        ("stale_redefined_in_loop", vec![], "другой вариант"),
        ("stale_lent_in_assign", vec![], "другой вариант"),
        ("stale_if_in_loop", vec![], "другой вариант"),
        ("stale_case_in_loop", vec![], "другой вариант"),
        ("stale_else_in_loop", vec![], "другой вариант"),
        ("stale_after_whole_write", vec![], "другой вариант"),
        ("local_empties", vec![], "в x нет значения"),
        ("local_unknown_type", vec![], "нет типа Q"),
        ("conv_result_type", vec![], "не подходит к типу i32"),
        ("borrow_wrong_type", vec![], "не подходит к параметру t"),
        ("lend_wrong_type", vec![], "не подходит к параметру t"),
        (
            "make_duplicate_field",
            vec![],
            "поля указаны не так, как объявлены",
        ),
        (
            "make_named_payload",
            vec![],
            "значения не подходят к объявлению варианта",
        ),
        ("shift_count_bool", vec![], "величина сдвига — целое"),
    ];
    for (func, args, fragment) in cases {
        invalid_with(call(&m, func, args), fragment);
    }
    assert_eq!(call(&m, "f32_exact", vec![]), Ok(Value::F32(0.1)));
    assert_eq!(call(&m, "half_ok", vec![]), Ok(Value::Unit));
    assert_eq!(call(&m, "no_case", vec![i64v(1)]), Ok(i64v(1)));
    // на другой ветке вариант не меняется: исполнение законно, отвергает только проверщик
    assert_eq!(
        call(&m, "stale_after_branch", vec![Value::Bool(false)]),
        Ok(i64v(5))
    );
    assert_eq!(
        call(&m, "stale_after_inner_switch", vec![i64v(1)]),
        Ok(i64v(5))
    );
}

#[test]
fn a_module_with_a_repeated_name_does_not_run() {
    let m = module("ir 1\n\nstruct A { }\n\nenum A { }\n");
    match interp::run_tests(&m, Limits::default()) {
        Err(Stop::InvalidIr(msg)) => assert!(msg.contains("объявлен дважды"), "{}", msg),
        other => panic!("{:?}", other),
    }
}
