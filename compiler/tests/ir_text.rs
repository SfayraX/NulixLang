//! Текстовая форма IR: запись и обратное чтение без потерь.
//!
//! Проверяется: канонические тексты читаются и записываются обратно буква в букву; примеры IR
//! из `spec/ir-core.md` записаны в канонической форме; случайные модули переживают запись и
//! чтение без изменений; раскладка и комментарии на прочитанное не влияют; неверный текст
//! отвергается с номером строки.

use nulixc::ir::*;
use nulixc::ir_text::{parse, print};

/// Текст читается, и запись прочитанного совпадает с ним буква в букву.
fn canonical(text: &str) {
    let m = match parse(text) {
        Ok(m) => m,
        Err(e) => panic!("строка {}: {}\n{}", e.line, e.message, text),
    };
    let again = print(&m);
    assert_eq!(again, text, "запись прочитанного отличается от текста");
    assert_eq!(parse(&again), Ok(m));
}

/// Все конструкции ядра шага 2.
const ALL_CONSTRUCTS: &str = r##"ir 1

struct Point { x: f64, y: f64 }

copy struct Pair { a: i64, b: i64 }

struct Empty { }

enum Shape { Circle(f64), Rect(w: f64, h: f64), Dot }

copy enum Dir { North, South }

enum Never { }

func gcd(a: u64, b: u64) -> u64 {
  x: u64 = copy a
  y: u64 = copy b
  loop L0 [held: x, y] {
    t0: bool = ne.u64(copy y, const 0)
    if t0 { } else {
      break L0
    } [held: x, y]
    r: u64 = rem.u64(copy x, copy y)
    x = copy y
    y = copy r
  } [held: x, y]
  return copy x
}

func area(s: Shape) -> f64 {
  switch s {
    case Circle {
      r: f64 = copy s.Circle.0
      t0: f64 = mul.f64(copy r, copy r)
      t1: f64 = mul.f64(const 3.141592653589793, copy t0)
      return copy t1
    }
    case Rect {
      t2: f64 = mul.f64(copy s.Rect.w, copy s.Rect.h)
      return copy t2
    }
    case Dot {
      return const 0.0
    }
  }
}

func fill(buf: Array[u8, 4] mut, v: u8) {
  i: i64 = const 0
  loop L1 [held: i] {
    t0: bool = lt.i64(copy i, const 4)
    if t0 { } else {
      break L1
    } [held: i]
    buf[i] = copy v
    i = add.i64(copy i, const 1)
    continue L1
  } [held: i]
  buf[0] = const 255
  return
}

func `swap`(p: Pair mut) {
  t0: i64 = copy p.a
  p.a = copy p.b
  p.b = copy t0
  swap p.a, p.b
}

func pack(p: Point take, d: Dir) -> (Point, Pair, Dir) {
  local q: Pair
  q = make Pair(a: const 1, b: const -2)
  q.b = neg.i64(copy q.a)
  call `swap`(borrow mut q)
  t0: (Point, Pair, Dir) = tuple(move p, copy q, copy d)
  return move t0
}

func shapes() -> Array[Shape, 3] {
  a: Shape = make Shape.Circle(const 1.5)
  b: Shape = make Shape.Rect(w: const 2.0, h: const 0.5)
  c: Shape = make Shape.Dot
  e: Empty = make Empty()
  drop e
  arr: Array[Shape, 3] = array(move a, move b, move c)
  return move arr
}

func note(b: bool) { }

func classify(x: i32, flag: bool, ch: char) -> f64 {
  t0: i64 = conv.i32.i64(copy x)
  t1: i64 = neg.i64(copy t0)
  t2: bool = not.bool(copy flag)
  t3: f64 = conv.i64.f64(copy t1)
  switch ch {
    case 'a' {
      call note(copy t2)
    }
    case '\n' {
      crash "перевод строки"
    }
    else { }
  } [held: t3]
  switch x {
    case -1 { }
    case 0 {
      t4: f64 = const -0.0
      return copy t4
    }
    else { }
  } [held: t3]
  switch flag {
    case true { }
    case false { }
  } [held: t3]
  return copy t3
}

func grid(g: Array[Array[i64, 2], 2]) -> i64 {
  t0: i64 = copy g[1][0]
  t1: (i64, i64) = tuple(copy t0, copy g[0][1])
  t2: ((i64, i64), ()) = tuple(copy t1, const ())
  return copy t2.0.1
}

test "gcd(12, 18) = 6" {
  t0: u64 = call gcd(const 12, const 18)
  t1: bool = eq.u64(copy t0, const 6)
  if t1 { } else {
    crash "assert"
  } [held:]
}
"##;

/// Имена в обратных кавычках, составные имена функций, экранирование в строках и символах.
const NAMES_AND_ESCAPES: &str = r##"ir 1

struct `struct` { `u8`: u8, имя: i64, `два слова`: bool, `1x`: char, `a.b`: f32 }

enum `held` { `true`, `case`(`struct`), Значение(`;`: i8) }

func `copy`.длина(self: `struct`, `move`: `held` take, xs: Array[i64, 3]) -> i64 uses (fs, net) {
  t0: i64 = copy self.имя
  t1: u8 = copy self.`u8`
  `case`: i64 = const 2
  `case` = const 1
  t2: i64 = copy xs[`case`]
  drop `move`
  switch t0 {
    case 1 { }
    else {
      crash "кавычка \" обратная черта \\ табуляция \t нуль \0 возврат \r звонок \u{7} конец"
    }
  } [held: t0]
  t3: char = const '\''
  t4: char = const '"'
  t5: char = const '\u{1b}'
  t6: char = const '🙂'
  return copy t0
}

func `Array`.`i64`(`borrow`: `Array`) {
  call `copy`.длина(borrow `borrow`)
}

test "" { }
"##;

#[test]
fn golden_texts_are_canonical() {
    canonical(ALL_CONSTRUCTS);
    canonical(NAMES_AND_ESCAPES);
    canonical("ir 1\n");
}

#[test]
fn spec_examples_are_canonical() {
    let path = format!("{}/../spec/ir-core.md", env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(&path).expect("не найден spec/ir-core.md");
    let mut blocks = 0;
    let mut rest = doc.as_str();
    while let Some(start) = rest.find("```ir\n") {
        let body = &rest[start + "```ir\n".len()..];
        let end = body.find("```").expect("блок ```ir не закрыт");
        canonical(&body[..end]);
        blocks += 1;
        rest = &body[end + 3..];
    }
    assert!(blocks >= 2, "в spec/ir-core.md меньше двух примеров IR");
}

#[test]
fn reading_ignores_layout_and_comments() {
    let loose = "ir 1 ; заголовок\nstruct   P{x:u8,y:u8}\nfunc `f`(a: P) -> () uses () { x: u8 = copy a.x ; комментарий\nreturn }\ntest \"\\u{41}\\'\" {}";
    let canon = "ir 1\n\nstruct P { x: u8, y: u8 }\n\nfunc f(a: P) {\n  x: u8 = copy a.x\n  return\n}\n\ntest \"A'\" { }\n";
    assert_eq!(parse(loose), parse(canon));
    canonical(canon);
}

#[test]
fn errors_name_the_line() {
    let cases: &[(&str, u32, &str)] = &[
        ("", 1, "ожидалось: «ir»; стоит конец текста"),
        ("ir 2\n", 1, "версия IR 2 не поддерживается"),
        ("ir 1\n\nstruct copy { }\n", 3, "«copy» — слово текстовой формы"),
        ("ir 1\n\ncopy func f() { }\n", 3, "«struct» или «enum» после «copy»"),
        (
            "ir 1\n\nfunc f() {\n  x: (u8) = const 1\n}\n",
            4,
            "у кортежа не меньше двух элементов",
        ),
        ("ir 1\n\nfunc f() {\n  x: S = make S\n}\n", 4, "make T(…)"),
        (
            "ir 1\n\nfunc f() {\n  x: i64 = neg.i64(copy a, copy b)\n}\n",
            4,
            "операция neg берёт операндов: 1; дано: 2",
        ),
        (
            "ir 1\n\nfunc f() {\n  x: (u8, u8) = tuple(copy a)\n}\n",
            4,
            "у кортежа не меньше двух элементов",
        ),
        ("ir 1\n\nfunc f() {\n  x: u8 = foo\n}\n", 4, "ожидалось: значение"),
        ("ir 1\n\nfunc f() {\n  swap a b\n}\n", 4, "ожидалось: «,»"),
        ("ir 1\n\nfunc f() {\n  x: u8 = copy u8\n}\n", 4, "«u8» — слово текстовой формы"),
        (
            "ir 1\n\nfunc f() {\n  switch x {\n    case 1.5 { }\n  }\n}\n",
            5,
            "вариант, целое, символ",
        ),
        (
            "ir 1\n\nfunc f() {\n  x: Array[u8, -1] = array()\n}\n",
            4,
            "вне допустимых пределов",
        ),
        ("ir 1\n\nfunc f() {\n  crash \"\\q\"\n}\n", 4, "неизвестное экранирование \\q"),
        ("ir 1\n\nfunc f() {\n  crash \"не закрыта\n}\n", 4, "кавычка не закрыта"),
        ("ir 1\n\nfunc f() {\n  x: f64 = const 1e400\n}\n", 4, "не помещается в f64"),
        (
            "ir 1\n\nfunc f() {\n  x: i64 = const 999999999999999999999999999999999999999999\n}\n",
            4,
            "слишком велико",
        ),
        ("ir 1\n\nfunc f() {\n  x: char = const 'ab'\n}\n", 4, "ровно один символ"),
        ("ir 1\n\nfunc f() {\n  x: char = const '\\u{d800}'\n}\n", 4, "не символ Unicode"),
        ("ir 1\n\nfunc f() {\n  x: u8 = const 1abc\n}\n", 4, "слито со словом"),
        ("ir 1\n\nfunc f() {\n  x: u8 = copy `a\n}\n", 4, "не закрыто"),
        ("ir 1\n\nfunc f() {\n  x: u8 = copy ``\n}\n", 4, "пустое имя"),
        ("ir 1\n\nfunc f() {\n  x: u8 = copy @\n}\n", 4, "неожиданный знак «@»"),
        ("ir 1\n\nfunc a.`b.c`() { }\n", 3, "часть имени функции не может содержать точку"),
        ("ir 1\n\nfunc f() {\n  x: u8 = copy a\n", 5, "ожидалось: «}»; стоит конец текста"),
        (
            "ir 1 ; заголовок\n; комментарий\n\nfunc f() {\n  ; ещё комментарий\n\n  return copy\n}\n",
            8,
            "ожидалось: имя",
        ),
    ];
    for (text, line, fragment) in cases {
        match parse(text) {
            Ok(_) => panic!("текст принят, а должен быть отвергнут:\n{}", text),
            Err(e) => {
                assert_eq!(
                    e.line, *line,
                    "строка ошибки «{}» в тексте:\n{}",
                    e.message, text
                );
                assert!(
                    e.message.contains(fragment),
                    "сообщение «{}» не содержит «{}»",
                    e.message,
                    fragment
                );
            }
        }
    }
}

// ───────── случайные модули ─────────

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

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Имена: обычные, похожие на слова формы, сами слова формы и скаляры, непригодные для
/// записи без кавычек.
const NAMES: &[&str] = &[
    "x",
    "y",
    "t0",
    "_",
    "a1",
    "имя",
    "Значение",
    "名前",
    "x_y",
    "self",
    "Str",
    "len",
    "é",
    "add",
    "neg",
    "ge",
    "Copy",
    "Held",
    "array2",
    "u128",
    "isize",
    "ir",
    "struct",
    "enum",
    "copy",
    "func",
    "test",
    "uses",
    "mut",
    "take",
    "local",
    "drop",
    "swap",
    "call",
    "if",
    "else",
    "switch",
    "case",
    "loop",
    "break",
    "continue",
    "return",
    "crash",
    "move",
    "borrow",
    "const",
    "make",
    "tuple",
    "array",
    "conv",
    "held",
    "true",
    "false",
    "Array",
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "f32",
    "f64",
    "bool",
    "char",
    "1x",
    "0",
    "два слова",
    "a-b",
    "a.b",
    ".",
    ";",
    "{",
    "}",
    "\"",
    "'",
    "()",
    "->",
    "[held:]",
    " ",
    "🙂",
    "x y z",
    "\u{a0}",
    "\u{2028}",
    "e\u{301}",
];

fn any_char(r: &mut Rng) -> char {
    const SPECIAL: &[char] = &[
        '\n',
        '\t',
        '\r',
        '\0',
        '\\',
        '\'',
        '"',
        '`',
        ';',
        '.',
        '\u{7}',
        '\u{1b}',
        '\u{7f}',
        '\u{85}',
        '\u{a0}',
        '\u{2028}',
        '\u{feff}',
        '🙂',
        'я',
        '\u{10ffff}',
    ];
    match r.below(3) {
        0 => *r.pick(SPECIAL),
        1 => (b' ' + r.below(95) as u8) as char,
        _ => loop {
            if let Some(c) = char::from_u32((r.next() % 0x11_0000) as u32) {
                return c;
            }
        },
    }
}

fn ident(r: &mut Rng) -> String {
    if !r.one_in(4) {
        return r.pick(NAMES).to_string();
    }
    loop {
        let len = 1 + r.below(5);
        let s: String = (0..len).map(|_| any_char(r)).collect();
        if !s.contains('`') && !s.chars().any(char::is_control) {
            return s;
        }
    }
}

/// Часть составного имени функции: без точки.
fn segment(r: &mut Rng) -> String {
    loop {
        let s = ident(r);
        if !s.contains('.') {
            return s;
        }
    }
}

fn func_name(r: &mut Rng) -> String {
    let n = 1 + r.below(3);
    (0..n).map(|_| segment(r)).collect::<Vec<_>>().join(".")
}

fn string(r: &mut Rng) -> String {
    let n = r.below(8);
    (0..n).map(|_| any_char(r)).collect()
}

fn int(r: &mut Rng) -> i128 {
    const EDGES: &[i128] = &[
        0,
        1,
        -1,
        255,
        -128,
        i128::MIN,
        i128::MAX,
        i64::MIN as i128,
        i64::MAX as i128,
        u64::MAX as i128,
    ];
    match r.below(5) {
        0 => *r.pick(EDGES),
        1 => r.next() as i128,
        2 => -(r.next() as i128),
        3 => (((r.next() as u128) << 64) | r.next() as u128) as i128,
        _ => r.below(100) as i128 - 50,
    }
}

/// Биты конечного f64.
fn float(r: &mut Rng) -> u64 {
    const EDGES: &[f64] = &[
        0.0,
        -0.0,
        1.0,
        -1.5,
        0.1,
        1e16,
        1e-7,
        1e300,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::EPSILON,
    ];
    match r.below(4) {
        0 => r.pick(EDGES).to_bits(),
        // субнормальные, обоих знаков
        1 => (r.below(1 << 20) as u64) | if r.one_in(2) { 1 << 63 } else { 0 },
        _ => loop {
            let bits = r.next();
            if f64::from_bits(bits).is_finite() {
                return bits;
            }
        },
    }
}

fn constant(r: &mut Rng) -> Const {
    match r.below(5) {
        0 => Const::Int(int(r)),
        1 => Const::Float(float(r)),
        2 => Const::Bool(r.one_in(2)),
        3 => Const::Char(any_char(r)),
        _ => Const::Unit,
    }
}

fn ty(r: &mut Rng, depth: u32) -> Type {
    let k = if depth == 0 { r.below(3) } else { r.below(5) };
    match k {
        0 => Type::Scalar(*r.pick(&SCALARS)),
        1 => Type::Unit,
        2 => Type::Named(ident(r)),
        3 => {
            let n = 2 + r.below(3);
            Type::Tuple((0..n).map(|_| ty(r, depth - 1)).collect())
        }
        _ => {
            let elem = ty(r, depth - 1);
            let len = if r.one_in(4) {
                u64::MAX
            } else {
                r.below(10) as u64
            };
            Type::Array(Box::new(elem), len)
        }
    }
}

fn place(r: &mut Rng) -> Place {
    let base = ident(r);
    let n = r.below(4);
    let path = (0..n)
        .map(|_| match r.below(4) {
            0 => Seg::Field(ident(r)),
            1 => Seg::Pos(if r.one_in(4) {
                u32::MAX
            } else {
                r.below(5) as u32
            }),
            2 => Seg::Elem(Index::Var(ident(r))),
            _ => Seg::Elem(Index::Lit(if r.one_in(4) {
                u64::MAX
            } else {
                r.below(10) as u64
            })),
        })
        .collect();
    Place { base, path }
}

fn operand(r: &mut Rng) -> Operand {
    match r.below(5) {
        0 => Operand::Copy(place(r)),
        1 => Operand::Move(place(r)),
        2 => Operand::Borrow(place(r)),
        3 => Operand::BorrowMut(place(r)),
        _ => Operand::Const(constant(r)),
    }
}

fn operands(r: &mut Rng, n: usize) -> Vec<Operand> {
    (0..n).map(|_| operand(r)).collect()
}

fn args(r: &mut Rng) -> Vec<Arg> {
    let n = r.below(4);
    (0..n)
        .map(|_| {
            let name = if r.one_in(3) { None } else { Some(ident(r)) };
            Arg {
                name,
                value: operand(r),
            }
        })
        .collect()
}

fn rhs(r: &mut Rng) -> Rhs {
    match r.below(7) {
        0 => Rhs::Use(operand(r)),
        1 => {
            let op = *r.pick(&OPS);
            let ty = *r.pick(&SCALARS);
            let args = operands(r, op.arity());
            Rhs::Op { op, ty, args }
        }
        2 => {
            let from = *r.pick(&SCALARS);
            let to = *r.pick(&SCALARS);
            Rhs::Conv {
                from,
                to,
                arg: operand(r),
            }
        }
        3 => {
            let func = func_name(r);
            let n = r.below(3);
            Rhs::Call {
                func,
                args: operands(r, n),
            }
        }
        4 => {
            let ty = ident(r);
            if r.one_in(2) {
                Rhs::Make {
                    ty,
                    variant: None,
                    args: Some(args(r)),
                }
            } else {
                let variant = Some(ident(r));
                let args = if r.one_in(2) { None } else { Some(args(r)) };
                Rhs::Make { ty, variant, args }
            }
        }
        5 => {
            let n = 2 + r.below(3);
            Rhs::Tuple(operands(r, n))
        }
        _ => {
            let n = r.below(4);
            Rhs::Array(operands(r, n))
        }
    }
}

fn state(r: &mut Rng) -> State {
    let n = r.below(4);
    State {
        held: (0..n).map(|_| place(r)).collect(),
    }
}

fn maybe_state(r: &mut Rng) -> Option<State> {
    if r.one_in(2) {
        None
    } else {
        Some(state(r))
    }
}

fn case_label(r: &mut Rng) -> CaseLabel {
    match r.below(4) {
        0 => CaseLabel::Variant(ident(r)),
        1 => CaseLabel::Value(Const::Int(int(r))),
        2 => CaseLabel::Value(Const::Char(any_char(r))),
        _ => CaseLabel::Value(Const::Bool(r.one_in(2))),
    }
}

fn block(r: &mut Rng, depth: u32) -> Block {
    let n = r.below(5);
    Block {
        stmts: (0..n).map(|_| stmt(r, depth)).collect(),
    }
}

fn stmt(r: &mut Rng, depth: u32) -> Stmt {
    let k = if depth == 0 { r.below(9) } else { r.below(12) };
    match k {
        0 => {
            let name = ident(r);
            Stmt::Local { name, ty: ty(r, 2) }
        }
        1 => {
            let name = ident(r);
            let ty = ty(r, 2);
            Stmt::Define {
                name,
                ty,
                value: rhs(r),
            }
        }
        2 => {
            let place = place(r);
            Stmt::Assign {
                place,
                value: rhs(r),
            }
        }
        3 => {
            if r.one_in(2) {
                Stmt::Drop(place(r))
            } else {
                Stmt::Swap(place(r), place(r))
            }
        }
        4 => {
            let func = func_name(r);
            let n = r.below(3);
            Stmt::Call {
                func,
                args: operands(r, n),
            }
        }
        5 => Stmt::Break(ident(r)),
        6 => Stmt::Continue(ident(r)),
        7 => Stmt::Return(if r.one_in(2) { None } else { Some(operand(r)) }),
        8 => Stmt::Crash(string(r)),
        9 => {
            let cond = place(r);
            let then = block(r, depth - 1);
            let otherwise = block(r, depth - 1);
            Stmt::If {
                cond,
                then,
                otherwise,
                after: maybe_state(r),
            }
        }
        10 => {
            let on = place(r);
            let n = r.below(4);
            let cases = (0..n)
                .map(|_| {
                    let label = case_label(r);
                    Case {
                        label,
                        body: block(r, depth - 1),
                    }
                })
                .collect();
            let otherwise = if r.one_in(2) {
                None
            } else {
                Some(block(r, depth - 1))
            };
            Stmt::Switch {
                on,
                cases,
                otherwise,
                after: maybe_state(r),
            }
        }
        _ => {
            let label = ident(r);
            let head = state(r);
            let body = block(r, depth - 1);
            Stmt::Loop {
                label,
                head,
                body,
                after: maybe_state(r),
            }
        }
    }
}

fn fields(r: &mut Rng, min: usize) -> Vec<Field> {
    let n = min + r.below(4);
    (0..n)
        .map(|_| {
            let name = ident(r);
            Field { name, ty: ty(r, 2) }
        })
        .collect()
}

fn decl(r: &mut Rng) -> Decl {
    match r.below(4) {
        0 => {
            let name = ident(r);
            let copy = r.one_in(2);
            Decl::Struct(StructDecl {
                name,
                copy,
                fields: fields(r, 0),
            })
        }
        1 => {
            let name = ident(r);
            let copy = r.one_in(2);
            let n = r.below(4);
            let variants = (0..n)
                .map(|_| {
                    let name = ident(r);
                    let payload = match r.below(3) {
                        0 => Payload::None,
                        1 => Payload::One(ty(r, 2)),
                        _ => Payload::Fields(fields(r, 1)),
                    };
                    Variant { name, payload }
                })
                .collect();
            Decl::Enum(EnumDecl {
                name,
                copy,
                variants,
            })
        }
        2 => {
            let name = func_name(r);
            let n = r.below(4);
            let params = (0..n)
                .map(|_| {
                    let name = ident(r);
                    let ty = ty(r, 2);
                    Param {
                        name,
                        ty,
                        mode: *r.pick(&[Mode::Read, Mode::Mut, Mode::Take]),
                    }
                })
                .collect();
            let result = if r.one_in(3) { Type::Unit } else { ty(r, 2) };
            let n = r.below(3);
            let uses = (0..n).map(|_| ident(r)).collect();
            Decl::Func(Func {
                name,
                params,
                result,
                uses,
                body: block(r, 3),
            })
        }
        _ => {
            let name = string(r);
            Decl::Test(Test {
                name,
                body: block(r, 3),
            })
        }
    }
}

fn module(r: &mut Rng) -> Module {
    let n = r.below(6);
    Module {
        version: VERSION,
        decls: (0..n).map(|_| decl(r)).collect(),
    }
}

#[test]
fn random_modules_survive_round_trip() {
    let mut r = Rng(0x4E55_4C49_5849_5231);
    for i in 0..2000 {
        let m = module(&mut r);
        let text = print(&m);
        let back = match parse(&text) {
            Ok(back) => back,
            Err(e) => panic!("модуль {}: строка {}: {}\n{}", i, e.line, e.message, text),
        };
        assert_eq!(
            back, m,
            "модуль {} прочитан не так, как записан:\n{}",
            i, text
        );
        assert_eq!(print(&back), text, "модуль {}: вторая запись отличается", i);

        // Одна порча: чтение не падает, а если текст всё же принят, прочитанное снова
        // переживает запись и чтение.
        let bad = spoil(&mut r, &text);
        if let Ok(other) = parse(&bad) {
            assert_eq!(
                parse(&print(&other)),
                Ok(other),
                "модуль {}: принятый испорченный текст не переживает запись:\n{}",
                i,
                bad
            );
        }
    }
}

/// Удаляет, повторяет или заменяет один символ текста.
fn spoil(r: &mut Rng, text: &str) -> String {
    const SIGNS: &[char] = &[
        '{', '}', '(', ')', '[', ']', ',', ':', '=', '.', '`', '"', '\'', ';', '\n', ' ', '-', '0',
        'a', '\\',
    ];
    let mut chars: Vec<char> = text.chars().collect();
    let at = r.below(chars.len());
    match r.below(3) {
        0 => {
            chars.remove(at);
        }
        1 => chars.insert(at, chars[at]),
        _ => chars[at] = *r.pick(SIGNS),
    }
    chars.into_iter().collect()
}
