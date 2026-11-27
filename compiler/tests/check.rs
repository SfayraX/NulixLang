//! Проверка явного ядра в исходнике (`spec/core-check.md`): файл, объявления и сигнатуры,
//! выражения и инструкции в телах, `match` — образцы, полнота и недостижимые ветви.
//!
//! Правильные объявления, тела и `match` принимаются, и проверенное дерево несёт их типы,
//! режимы и простоту. Каждое правило нарушается по одному разу в отдельной программе, и проверка
//! отвергает её с ожидаемыми строкой и сообщением.

use nulixc::check::check;
use nulixc::core::{Payload, Program, Type, TypeKind};
use nulixc::ir::{Mode, Scalar};
use nulixc::parser::parse_file;
use nulixc::stack;

// ───────── помощники ─────────

fn run(text: &str) -> Result<Program, (u32, String)> {
    let file = match parse_file(text) {
        Ok(file) => file,
        Err(e) => panic!("разбор, строка {}: {}\n{}", e.line, e.message, text),
    };
    check(&file).map_err(|e| (e.line, e.message))
}

fn accepted(text: &str) -> Program {
    match run(text) {
        Ok(program) => program,
        Err((line, message)) => panic!("отвергнуто, строка {}: {}\n{}", line, message, text),
    }
}

fn rejected(text: &str, line: u32, message: &str) {
    match run(text) {
        Ok(_) => panic!("принято, а ждали «{}»:\n{}", message, text),
        Err((l, m)) => assert_eq!((l, m.as_str()), (line, message), "\n{}", text),
    }
}

fn type_id(p: &Program, name: &str) -> usize {
    match p.types.iter().position(|t| t.name == name) {
        Some(id) => id,
        None => panic!("нет типа {}", name),
    }
}

const I64: Type = Type::Scalar(Scalar::I64);

/// Номер первой строки текста, в которой есть `needle`.
fn line_of(text: &str, needle: &str) -> u32 {
    match text.lines().position(|l| l.contains(needle)) {
        Some(k) => k as u32 + 1,
        None => panic!("в тексте нет «{}»", needle),
    }
}

/// Проверка на потоке с маленьким стеком: так видно, что обход не рекурсивный.
fn run_on_small_stack(text: String) -> Result<Program, (u32, String)> {
    let handle = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || run(&text))
        .expect("не удалось создать поток");
    match handle.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

// ───────── правильные объявления ─────────

const DECLS: &str = "@file(develop)

/// Точка на плоскости.
struct Point {
\tx: i64
\ty: i64
}

impl Copy for Point {}

enum Shape {
\tCircle(r: f64)
\tRect(w: f64,
\t\th: f64)
\tEmpty
}

enum Op {
\tAdd
\tSub
}

impl Copy for Op {}

struct Line {
\tfrom: Point
\tto: Point
\ttag: (u8, char)
\tmarks: Array[Point, 4]
\top: Op
}

impl Copy for Line {
} end impl Copy for Line

struct Token {
\tid: i64
}

struct Pair {
\ta: Token
\tb: Token
\tcount: Array[Array[u16, 0x10], 0]
}

enum Void {}

enum Wrap {
\tOne(Token)
\tUnit(())
}

@system
pub func area(s: Shape) -> f64 {
\t0.0
} end area

func move_by(mut p: Point, dx: i64) {
}

func consume(take t: Token) -> Token {
\tt
}

@high
func forever() -> Never {
\tloop {}
}

func unit() -> () {
}

func pair_of(a: i64, b: (bool, f32)) -> (i64, (bool, f32)) {
\t(a, b)
}

test \"площадь\" {
}

test \"имя с \\\"кавычками\\\" и \\$\" {
} end test
";

#[test]
fn declarations_are_accepted() {
    let p = accepted(DECLS);
    let names: Vec<&str> = p.types.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["Point", "Shape", "Op", "Line", "Token", "Pair", "Void", "Wrap"]
    );
    let copy: Vec<&str> = p
        .types
        .iter()
        .filter(|t| t.copy)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(copy, ["Point", "Op", "Line"]);

    let point = type_id(&p, "Point");
    let line = &p.types[type_id(&p, "Line")];
    match &line.kind {
        TypeKind::Struct(fields) => {
            let texts: Vec<String> = fields.iter().map(|f| p.type_text(&f.ty)).collect();
            assert_eq!(
                texts,
                ["Point", "Point", "(u8, char)", "Array[Point, 4]", "Op"]
            );
            assert_eq!(fields[3].ty, Type::Array(Box::new(Type::Named(point)), 4));
        }
        TypeKind::Enum(_) => panic!("Line — запись"),
    }
    let pair = &p.types[type_id(&p, "Pair")];
    match &pair.kind {
        TypeKind::Struct(fields) => {
            assert_eq!(p.type_text(&fields[2].ty), "Array[Array[u16, 16], 0]")
        }
        TypeKind::Enum(_) => panic!("Pair — запись"),
    }
    match &p.types[type_id(&p, "Shape")].kind {
        TypeKind::Enum(variants) => {
            let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
            assert_eq!(names, ["Circle", "Rect", "Empty"]);
            match &variants[1].payload {
                Payload::Fields(fields) => {
                    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
                    assert_eq!(names, ["w", "h"]);
                    assert_eq!(fields[0].ty, Type::Scalar(Scalar::F64));
                }
                other => panic!("у Rect поля, а не {:?}", other),
            }
            assert_eq!(variants[2].payload, Payload::None);
        }
        TypeKind::Struct(_) => panic!("Shape — перечисление"),
    }
    match &p.types[type_id(&p, "Wrap")].kind {
        TypeKind::Enum(variants) => {
            assert_eq!(
                variants[0].payload,
                Payload::One(Type::Named(type_id(&p, "Token")))
            );
            assert_eq!(variants[1].payload, Payload::One(Type::Unit));
        }
        TypeKind::Struct(_) => panic!("Wrap — перечисление"),
    }

    // простота: скаляры, кортежи и массивы из простых, типы с impl Copy, Never (10.2, 12.27)
    let token = Type::Named(type_id(&p, "Token"));
    assert!(p.is_simple(&I64));
    assert!(p.is_simple(&Type::Unit));
    assert!(p.is_simple(&Type::Never));
    assert!(p.is_simple(&Type::Named(point)));
    assert!(p.is_simple(&Type::Tuple(vec![I64, Type::Named(point)])));
    assert!(!p.is_simple(&token));
    assert!(!p.is_simple(&Type::Tuple(vec![I64, token.clone()])));
    assert!(!p.is_simple(&Type::Array(Box::new(token.clone()), 0)));
    assert!(!p.is_simple(&Type::Named(type_id(&p, "Pair"))));
    // типы без значений: Never и любое перечисление без вариантов (9.36)
    assert!(p.is_never(&Type::Never));
    assert!(p.is_never(&Type::Named(type_id(&p, "Void"))));
    assert!(!p.is_never(&Type::Named(type_id(&p, "Op"))));
    assert!(!p.is_never(&Type::Unit));

    let funcs: Vec<&str> = p.funcs.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        funcs,
        ["area", "move_by", "consume", "forever", "unit", "pair_of"]
    );
    let area = &p.funcs[0];
    assert_eq!(area.params[0].ty, Type::Named(type_id(&p, "Shape")));
    assert_eq!(area.params[0].mode, Mode::Read);
    assert_eq!(area.result, Type::Scalar(Scalar::F64));
    let move_by = &p.funcs[1];
    let modes: Vec<Mode> = move_by.params.iter().map(|q| q.mode).collect();
    assert_eq!(modes, [Mode::Mut, Mode::Read]);
    assert_eq!(move_by.result, Type::Unit);
    assert_eq!(p.funcs[2].params[0].mode, Mode::Take);
    assert_eq!(p.funcs[2].result, token);
    assert_eq!(p.funcs[3].result, Type::Never);
    assert_eq!(p.funcs[4].result, Type::Unit);
    assert_eq!(p.type_text(&p.funcs[5].result), "(i64, (bool, f32))");

    let tests: Vec<&str> = p.tests.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tests, ["площадь", "имя с \"кавычками\" и $"]);

    // строки: у типа — слово struct или enum, у поля и варианта — их строка, у функции —
    // слово func, у параметра — его строка, у теста — строка имени
    assert_eq!(p.types[point].line, line_of(DECLS, "struct Point"));
    assert_eq!(line.line, line_of(DECLS, "struct Line"));
    match &line.kind {
        TypeKind::Struct(fields) => {
            assert_eq!(fields[0].line, line_of(DECLS, "\tfrom: Point"));
            assert_eq!(fields[4].line, line_of(DECLS, "\top: Op"));
        }
        TypeKind::Enum(_) => panic!("Line — запись"),
    }
    let shape = &p.types[type_id(&p, "Shape")];
    assert_eq!(shape.line, line_of(DECLS, "enum Shape"));
    match &shape.kind {
        TypeKind::Enum(variants) => {
            assert_eq!(variants[0].line, line_of(DECLS, "\tCircle("));
            assert_eq!(variants[2].line, line_of(DECLS, "\tEmpty"));
            match &variants[1].payload {
                Payload::Fields(fields) => {
                    assert_eq!(fields[0].line, line_of(DECLS, "\tRect("));
                    assert_eq!(fields[1].line, line_of(DECLS, "\t\th: f64)"));
                }
                other => panic!("у Rect поля, а не {:?}", other),
            }
        }
        TypeKind::Struct(_) => panic!("Shape — перечисление"),
    }
    assert_eq!(p.funcs[0].line, line_of(DECLS, "pub func area"));
    assert_eq!(p.funcs[3].line, line_of(DECLS, "func forever"));
    assert_eq!(move_by.params[1].line, line_of(DECLS, "func move_by"));
    assert_eq!(p.tests[0].line, line_of(DECLS, "test \"площадь\""));
    assert_eq!(p.tests[1].line, line_of(DECLS, "test \"имя с"));
}

#[test]
fn array_lengths_are_read_in_every_base() {
    let p = accepted(
        "func f(a: Array[u8, 0b101], b: Array[u8, 0o17], c: Array[u8, 0x1F], d: Array[u8, 1_000]) {\n}\n",
    );
    let lengths: Vec<u64> = p.funcs[0]
        .params
        .iter()
        .map(|q| match &q.ty {
            Type::Array(_, n) => *n,
            other => panic!("не массив: {:?}", other),
        })
        .collect();
    assert_eq!(lengths, [5, 15, 31, 1000]);
}

#[test]
fn test_names_are_read_with_every_escape() {
    let p = accepted("test \"\\n\\t\\r\\0\\\\\\\"\\'\\$\\u{41}\\u{1F600}\" {\n}\n");
    assert_eq!(p.tests[0].name, "\n\t\r\0\\\"'$A\u{1F600}");
    // строка теста — строка его имени, а не пометки над ним
    let p = accepted("@high\ntest \"a\" {\n}\n");
    assert_eq!(p.tests[0].line, 2);
}

#[test]
fn every_strictness_word_is_accepted() {
    for word in ["prototype", "develop", "production", "strict"] {
        accepted(&format!("@file({})\nfunc f() {{\n}}\n", word));
    }
    accepted("func f() {\n}\n");
}

#[test]
fn order_of_declarations_does_not_matter() {
    // тип объявлен ниже, чем используется, и impl Copy стоит раньше своего типа
    let p = accepted(
        "impl Copy for B {}\nfunc f(a: A) -> B {\n\tB(x: 1)\n}\nstruct A {\n\tb: B\n}\nimpl Copy for A {}\nstruct B {\n\tx: i64\n}\n",
    );
    assert!(p.types.iter().all(|t| t.copy));
}

#[test]
fn types_without_cycles_are_accepted() {
    // ромб: A содержит B и C, оба содержат D — цикла нет
    accepted("struct A {\n\tb: B\n\tc: C\n}\nstruct B {\n\td: D\n}\nstruct C {\n\td: D\n}\nstruct D {\n}\n");
}

#[test]
fn longest_array_is_accepted() {
    let p = accepted("func f(a: Array[u8, 9_223_372_036_854_775_807]) {\n}\n");
    assert_eq!(
        p.funcs[0].params[0].ty,
        Type::Array(Box::new(Type::Scalar(Scalar::U8)), i64::MAX as u64)
    );
}

#[test]
fn long_chain_of_types_is_checked_without_recursion() {
    // 20 000 записей одна в другой: обход типов идёт по явному стеку
    let n = 20_000;
    let mut text = String::new();
    for k in 0..n {
        text.push_str(&format!("struct T{} {{\n\tnext: T{}\n}}\n", k, k + 1));
    }
    text.push_str(&format!("struct T{} {{\n}}\n", n));
    let p = match run_on_small_stack(text.clone()) {
        Ok(p) => p,
        Err(e) => panic!("отвергнуто: {:?}", e),
    };
    assert_eq!(p.types.len(), n + 1);
    // тот же ряд, замкнутый в кольцо, отвергается
    let ring = text.replace(
        &format!("struct T{} {{\n}}\n", n),
        &format!("struct T{} {{\n\tnext: T0\n}}\n", n),
    );
    match run_on_small_stack(ring) {
        Err((line, message)) => assert_eq!(
            (line, message.as_str()),
            (
                1,
                "тип T0 содержит сам себя без косвенности: T0 → T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8 → … → T0 (9.9)"
            )
        ),
        Ok(_) => panic!("кольцо из типов принято"),
    }
}

#[test]
fn deeply_nested_types_are_checked() {
    // кортеж в кортеже на 250 уровней — у предела вложенности разборщика (256)
    let depth = 250;
    let mut ty = "i64".to_string();
    for _ in 0..depth {
        ty = format!("({}, u8)", ty);
    }
    let text = format!("func f(x: {}) {{\n}}\n", ty);
    let p = stack::run_scoped(|| accepted(&text));
    assert!(p.is_simple(&p.funcs[0].params[0].ty));
}

// ───────── отвергнутые объявления ─────────

/// Программа, строка ошибки и сообщение.
const REJECTED: &[(&str, u32, &str)] = &[
    // заголовок @file (решение 10м)
    (
        "@file(production, target(x86))\n",
        1,
        "не входит в шаг 2: target(…) в заголовке @file",
    ),
    ("@file(fast)\n", 1, "@file: неизвестная строгость fast"),
    (
        "@file(production, strict)\n",
        1,
        "@file: строгость указана дважды",
    ),
    (
        "// заголовок ниже\n\n@file(oops)\n",
        3,
        "@file: неизвестная строгость oops",
    ),
    // конструкции верхнего уровня вне шага 2 (раздел 3)
    ("import net.tcp\n", 1, "не входит в шаг 2: импорт"),
    (
        "f()\n",
        1,
        "не входит в шаг 2: инструкции на верхнем уровне",
    ),
    ("let x = 1\n", 1, "не входит в шаг 2: let на верхнем уровне"),
    ("var x = 1\n", 1, "не входит в шаг 2: var на верхнем уровне"),
    (
        "const C: i64 = 1\n",
        1,
        "не входит в шаг 2: const на верхнем уровне",
    ),
    (
        "extern \"C\" func f() {\n}\n",
        1,
        "не входит в шаг 2: extern",
    ),
    (
        "extern \"C\" trust \"сверено\" {\n\tfunc g()\n}\n",
        1,
        "не входит в шаг 2: extern",
    ),
    ("interface I {\n}\n", 1, "не входит в шаг 2: interface"),
    ("type T = i64\n", 1, "не входит в шаг 2: type"),
    // пометки объявлений: принимаются только @high и @system (решение 10м)
    ("@low\nfunc f() {\n}\n", 1, "не входит в шаг 2: @low"),
    ("@naked\nfunc f() {\n}\n", 1, "не входит в шаг 2: @naked"),
    (
        "@production\nfunc f() {\n}\n",
        1,
        "не входит в шаг 2: пометка строгости @production у объявления",
    ),
    (
        "@layout(c)\nstruct S {\n}\n",
        1,
        "не входит в шаг 2: пометка @layout",
    ),
    // реализации: только impl Copy for T {} (12.27)
    (
        "struct P {\n}\nimpl P {\n}\n",
        3,
        "не входит в шаг 2: impl, кроме impl Copy for T {}",
    ),
    (
        "struct P {\n}\nimpl Display for P {\n}\n",
        3,
        "не входит в шаг 2: impl, кроме impl Copy for T {}",
    ),
    (
        "struct P {\n}\nimpl[T] Copy for P {\n}\n",
        3,
        "не входит в шаг 2: impl, кроме impl Copy for T {}",
    ),
    (
        "struct P {\n}\nimpl Copy[i64] for P {\n}\n",
        3,
        "не входит в шаг 2: impl, кроме impl Copy for T {}",
    ),
    (
        "impl Copy {\n}\n",
        1,
        "не входит в шаг 2: impl, кроме impl Copy for T {}",
    ),
    // функции
    ("func f[T](x: T) {\n}\n", 1, "не входит в шаг 2: обобщения"),
    ("func f() uses net {\n}\n", 1, "не входит в шаг 2: uses"),
    (
        "func f() without crash {\n}\n",
        1,
        "не входит в шаг 2: without",
    ),
    (
        "func f() limit stack <= 4096 {\n}\n",
        1,
        "не входит в шаг 2: limit",
    ),
    ("func f(self) {\n}\n", 1, "не входит в шаг 2: методы и self"),
    ("func f(x) {\n}\n", 1, "параметр x: тип не указан"),
    ("func f(a: i64,\n\ta: u8) {\n}\n", 2, "параметр a повторён"),
    (
        "func f() {\n} end g\n",
        1,
        "метка end g не совпадает с именем функции f",
    ),
    // записи
    (
        "struct S[T] {\n\tx: T\n}\n",
        1,
        "не входит в шаг 2: обобщения",
    ),
    (
        "struct S {\n\t@offset(0) x: u32\n}\n",
        2,
        "не входит в шаг 2: пометки полей",
    ),
    (
        "struct S {\n\tx: i64 = 0\n}\n",
        2,
        "не входит в шаг 2: значения полей по умолчанию",
    ),
    ("struct S {\n\tx: i64\n\tx: u8\n}\n", 3, "поле x повторено"),
    (
        "struct S {\n} end T\n",
        1,
        "метка end T не совпадает с именем записи S",
    ),
    // перечисления
    (
        "enum E[T] {\n\tA(T)\n}\n",
        1,
        "не входит в шаг 2: обобщения",
    ),
    (
        "enum E {\n\tindirect A(E)\n}\n",
        2,
        "не входит в шаг 2: indirect",
    ),
    ("enum E {\n\tA\n\tB\n\tA\n}\n", 4, "вариант A повторён"),
    (
        "enum E {\n\tA(x: i64, x: u8)\n}\n",
        2,
        "поле x варианта A повторено",
    ),
    (
        "enum E {\n\tA(x: i64,\n\t\tx: u8)\n}\n",
        3,
        "поле x варианта A повторено",
    ),
    (
        "enum E {\n} end F\n",
        1,
        "метка end F не совпадает с именем перечисления E",
    ),
    // имена объявлений: одно пространство имён у функций и типов (раздел 4)
    (
        "struct A {\n}\nfunc A() {\n}\n",
        3,
        "имя A уже объявлено в строке 1",
    ),
    (
        "func f() {\n}\nenum f {\n}\n",
        3,
        "имя f уже объявлено в строке 1",
    ),
    (
        "func f() {\n}\n\nfunc f() {\n}\n",
        4,
        "имя f уже объявлено в строке 1",
    ),
    (
        "struct A {\n}\n@system\npub struct A {\n}\n",
        4,
        "имя A уже объявлено в строке 1",
    ),
    (
        "@high\nenum E {\n} end F\n",
        2,
        "метка end F не совпадает с именем перечисления E",
    ),
    (
        "struct P {\n}\n@system\nimpl Copy for P {\n\tfunc f() {\n\t}\n}\n",
        4,
        "impl Copy for P пишется пустым (12.27)",
    ),
    // встроенные типы (решение 10и)
    ("struct i64 {\n}\n", 1, "имя i64 занято встроенным типом"),
    ("enum Never {\n}\n", 1, "имя Never занято встроенным типом"),
    (
        "func Array() {\n}\n",
        1,
        "имя Array занято встроенным типом",
    ),
    ("func char() {\n}\n", 1, "имя char занято встроенным типом"),
    // стандартные имена (решение 17)
    (
        "struct Copy {\n}\n",
        1,
        "имя Copy занято: оно доступно без импорта (12.89–12.91)",
    ),
    (
        "func print() {\n}\n",
        1,
        "имя print занято: оно доступно без импорта (12.89–12.91)",
    ),
    (
        "enum Option {\n}\n",
        1,
        "имя Option занято: оно доступно без импорта (12.89–12.91)",
    ),
    (
        "struct Add {\n}\n",
        1,
        "имя Add занято: оно доступно без импорта (12.89–12.91)",
    ),
    // типы
    ("func f(x: Str) {\n}\n", 1, "не входит в шаг 2: Str"),
    (
        "func f(x: Option[i64]) {\n}\n",
        1,
        "не входит в шаг 2: Option",
    ),
    ("func f(x: List[i64]) {\n}\n", 1, "не входит в шаг 2: List"),
    ("func f() -> Str {\n}\n", 1, "не входит в шаг 2: Str"),
    ("func f(x: Unknown) {\n}\n", 1, "нет типа Unknown"),
    ("func f(x: a.b) {\n}\n", 1, "нет типа a.b"),
    ("func f(x: f) {\n}\n", 1, "нет типа f"),
    ("func f(x: print) {\n}\n", 1, "нет типа print"),
    ("func f(x: Some) {\n}\n", 1, "нет типа Some"),
    ("func f(x: Copy) {\n}\n", 1, "нет типа Copy"),
    ("func f(x: Bytes) {\n}\n", 1, "не входит в шаг 2: Bytes"),
    (
        "struct P {\n}\nfunc f(x: P[i64]) {\n}\n",
        3,
        "не входит в шаг 2: обобщения",
    ),
    (
        "func f(x: i64[3]) {\n}\n",
        1,
        "тип i64 пишется без параметров",
    ),
    (
        "func f(x: Never[i64]) {\n}\n",
        1,
        "тип Never пишется без параметров",
    ),
    ("func f(x: any) {\n}\n", 1, "не входит в шаг 2: any"),
    ("func f(x: ref i64) {\n}\n", 1, "не входит в шаг 2: ref"),
    ("func f(x: ptr u8) {\n}\n", 1, "не входит в шаг 2: ptr"),
    (
        "func f(g: func(i64) -> i64) {\n}\n",
        1,
        "не входит в шаг 2: типы функций",
    ),
    ("func f(x: (i64, Str)) {\n}\n", 1, "не входит в шаг 2: Str"),
    (
        "func f(x: Array[i64]) {\n}\n",
        1,
        "массив пишется Array[T, N]: тип элемента и длина-число",
    ),
    (
        "func f(x: Array[i64, n]) {\n}\n",
        1,
        "массив пишется Array[T, N]: тип элемента и длина-число",
    ),
    (
        "func f(x: Array[3, i64]) {\n}\n",
        1,
        "массив пишется Array[T, N]: тип элемента и длина-число",
    ),
    (
        "func f(x: Array[i64, 9223372036854775808]) {\n}\n",
        1,
        "длина массива 9223372036854775808 больше наибольшего i64 (12.19)",
    ),
    (
        "func f(x: Array[Str, 2]) {\n}\n",
        1,
        "не входит в шаг 2: Str",
    ),
    ("struct S {\n\tx: Q\n}\n", 2, "нет типа Q"),
    ("enum E {\n\tA(Q)\n}\n", 2, "нет типа Q"),
    ("enum E {\n\tA(x: i64, y: Q)\n}\n", 2, "нет типа Q"),
    ("func f() -> Q {\n}\n", 1, "нет типа Q"),
    // типы, которые содержат сами себя (9.9; правило 5 проверщика IR)
    (
        "struct A {\n\ta: A\n}\n",
        1,
        "тип A содержит сам себя без косвенности: A → A (9.9)",
    ),
    (
        "struct A {\n\tb: B\n}\nstruct B {\n\ta: (i64, A)\n}\n",
        1,
        "тип A содержит сам себя без косвенности: A → B → A (9.9)",
    ),
    (
        "enum L {\n\tNil\n\tCons(Array[L, 0])\n}\n",
        1,
        "тип L содержит сам себя без косвенности: L → L (9.9)",
    ),
    (
        "enum L {\n\tNil\n\tCons(head: i64, tail: L)\n}\n",
        1,
        "тип L содержит сам себя без косвенности: L → L (9.9)",
    ),
    (
        "struct A {\n\tb: B\n}\nstruct B {\n\tc: C\n}\nstruct C {\n\tb: B\n}\n",
        4,
        "тип B содержит сам себя без косвенности: B → C → B (9.9)",
    ),
    // impl Copy for T {} (12.27, 10.2)
    (
        "impl Copy for i64 {\n}\n",
        1,
        "impl Copy for i64: Copy объявляют только у записей и перечислений пакета",
    ),
    (
        "impl Copy for Never {\n}\n",
        1,
        "impl Copy for Never: Copy объявляют только у записей и перечислений пакета",
    ),
    ("impl Copy for Q {\n}\n", 1, "нет типа Q"),
    ("func f() {\n}\nimpl Copy for f {\n}\n", 3, "нет типа f"),
    ("struct P {\n}\nimpl Copy for a.P {\n}\n", 3, "нет типа a.P"),
    (
        "struct P {\n}\nimpl Copy for P[i64] {\n}\n",
        3,
        "не входит в шаг 2: обобщения",
    ),
    (
        "struct P {\n}\nimpl Copy for P {\n}\nimpl Copy for P {\n}\n",
        5,
        "impl Copy for P повторён",
    ),
    (
        "struct P {\n}\nimpl Copy for P {\n\tfunc f() {\n\t}\n}\n",
        3,
        "impl Copy for P пишется пустым (12.27)",
    ),
    (
        "struct P {\n}\nimpl Copy for P {\n} end impl Copy for Q\n",
        3,
        "метка end impl Copy for Q не совпадает с заголовком impl Copy for P",
    ),
    (
        "struct P {\n}\nimpl Copy for P {\n} end impl Display for P\n",
        3,
        "метка end impl Display for P не совпадает с заголовком impl Copy for P",
    ),
    (
        "struct P {\n}\nimpl Copy for P {\n} end impl Copy\n",
        3,
        "метка end impl Copy не совпадает с заголовком impl Copy for P",
    ),
    (
        "impl Copy for Str {\n}\n",
        1,
        "impl Copy for Str: Copy объявляют только у записей и перечислений пакета",
    ),
    (
        "impl Copy for Array[i64, 3] {\n}\n",
        1,
        "impl Copy for Array: Copy объявляют только у записей и перечислений пакета",
    ),
    (
        "struct T {\n}\nstruct P {\n\tt: T\n}\nimpl Copy for P {\n}\n",
        6,
        "impl Copy for P: поле t типа T не простое",
    ),
    (
        "struct T {\n}\nstruct P {\n\tt: Array[T, 2]\n}\nimpl Copy for P {\n}\n",
        6,
        "impl Copy for P: поле t типа Array[T, 2] не простое",
    ),
    (
        "struct T {\n}\nenum E {\n\tA(T)\n}\nimpl Copy for E {\n}\n",
        6,
        "impl Copy for E: значение варианта A типа T не простое",
    ),
    (
        "struct T {\n}\nenum E {\n\tA(x: (i64, T))\n}\nimpl Copy for E {\n}\n",
        6,
        "impl Copy for E: поле x варианта A типа (i64, T) не простое",
    ),
    (
        "struct A {\n\tb: B\n}\nimpl Copy for A {\n}\nstruct B {\n\tx: i64\n}\n",
        4,
        "impl Copy for A: поле b типа B не простое",
    ),
    // тесты: имя — строка без подстановок, имена не повторяются (решения 10л и 19)
    (
        "test \"a $x\" {\n}\n",
        1,
        "имя теста — строка без подстановок",
    ),
    (
        "test \"\"\"многострочное\"\"\" {\n}\n",
        1,
        "не входит в шаг 2: многострочная строка в имени теста",
    ),
    (
        "test \"a\" {\n}\ntest \"a\" {\n}\n",
        3,
        "тест «a» уже объявлен в строке 1",
    ),
    (
        "test \"a\" {\n}\ntest \"\\u{61}\" {\n}\n",
        3,
        "тест «a» уже объявлен в строке 1",
    ),
    (
        "test \"a\\nb\" {\n}\ntest \"a\\nb\" {\n}\n",
        3,
        "тест «a\\nb» уже объявлен в строке 1",
    ),
    (
        "test \"\\u{D800}\" {\n}\n",
        1,
        "\\u{D800} — не скалярное значение Unicode",
    ),
    (
        "test \"\\u{110000}\" {\n}\n",
        1,
        "\\u{110000} — не скалярное значение Unicode",
    ),
];

#[test]
fn wrong_declarations_are_rejected() {
    for (text, line, message) in REJECTED {
        rejected(text, *line, message);
    }
}

#[test]
fn equal_test_names_are_compared_by_value() {
    // разные записи одного имени — повтор; разные имена — нет
    accepted("test \"a\" {\n}\ntest \"b\" {\n}\ntest \"a \" {\n}\n");
    let p = accepted("test \"\\t\\u{1F600}\\\\\" {\n}\n");
    assert_eq!(p.tests[0].name, "\t\u{1F600}\\");
}

// ───────── тела функций: правильная программа ─────────

use nulixc::core::{Block, Expr, ExprKind, Func, StmtKind};

const BODIES: &str = include_str!("check/bodies.nx");

fn func<'p>(p: &'p Program, name: &str) -> &'p Func {
    match p.funcs.iter().find(|f| f.name == name) {
        Some(f) => f,
        None => panic!("нет функции {}", name),
    }
}

/// Имена и типы локальных имён функции по порядку.
fn local_types(p: &Program, name: &str) -> Vec<(String, String)> {
    func(p, name)
        .body
        .locals
        .iter()
        .map(|l| (l.name.clone(), p.type_text(&l.ty)))
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// Номера циклов, на которые указывают `break` в блоке и во вложенных блоках.
fn breaks(b: &Block, out: &mut Vec<usize>) {
    for st in &b.stmts {
        match &st.kind {
            StmtKind::Break(id) => out.push(*id),
            StmtKind::While(_, _, body) | StmtKind::Loop(_, body) => breaks(body, out),
            StmtKind::Expr(e) => expr_breaks(e, out),
            _ => {}
        }
    }
}

fn expr_breaks(e: &Expr, out: &mut Vec<usize>) {
    if let ExprKind::If(branches, otherwise) = &e.kind {
        for (_, b) in branches {
            breaks(b, out);
        }
        if let Some(b) = otherwise {
            breaks(b, out);
        }
    }
}

#[test]
fn bodies_program_is_accepted() {
    let p = accepted(BODIES);
    // тип переменной — из правой части (решение 1); литерал берёт тип другого операнда или
    // ветви со своим типом (решение 2)
    assert_eq!(
        local_types(&p, "mix"),
        pairs(&[("a", "u8"), ("b", "u8"), ("s", "u8"), ("wide", "u16")])
    );
    assert_eq!(
        local_types(&p, "pick"),
        pairs(&[("flag", "bool"), ("small", "u8"), ("v", "u8")])
    );
    assert_eq!(
        local_types(&p, "limits"),
        pairs(&[("lo", "i8"), ("hi", "u64"), ("f", "f32"), ("c", "char")])
    );
    assert_eq!(
        local_types(&p, "find")[2..],
        pairs(&[("row", "i64"), ("found", "(i64, i64)"), ("col", "i64")])[..]
    );
    assert_eq!(
        local_types(&p, "swap_pair"),
        pairs(&[("p", "(i64, bool)"), ("n", "i64"), ("flag", "bool")])
    );
    // ветвь без значений тип не задаёт: литерал в другой ветви — i64
    assert_eq!(
        local_types(&p, "sign"),
        pairs(&[("x", "i64"), ("s", "i64")])
    );

    // значения литералов: минус — часть литерала, дробное округлено до своего типа
    let limits = &func(&p, "limits").body.block;
    match &limits.stmts[0].kind {
        StmtKind::Let(_, e) => assert_eq!(e.kind, ExprKind::Int(-128)),
        other => panic!("ждали let, а здесь {:?}", other),
    }
    match &limits.stmts[2].kind {
        StmtKind::Let(_, e) => {
            assert_eq!(e.kind, ExprKind::Float(1500.0));
            assert_eq!(p.type_text(&e.ty), "f32");
        }
        other => panic!("ждали let, а здесь {:?}", other),
    }

    // break outer указывает на внешний цикл
    let find = &func(&p, "find").body.block;
    let outer = find
        .stmts
        .iter()
        .find_map(|st| match &st.kind {
            StmtKind::While(id, _, _) => Some(*id),
            _ => None,
        })
        .unwrap_or(usize::MAX);
    let mut ids = Vec::new();
    breaks(find, &mut ids);
    assert_eq!(ids, [outer]);

    // тела, которые всегда уходят, имеют тип Never (9.36)
    for name in ["forever", "nothing"] {
        assert_eq!(func(&p, name).body.block.ty, Type::Never, "{}", name);
    }
    // поля записи — в порядке записи: так они и вычисляются (9.27)
    match func(&p, "make_token")
        .body
        .block
        .value
        .as_deref()
        .map(|e| &e.kind)
    {
        Some(ExprKind::Record(_, fields)) => {
            let order: Vec<usize> = fields.iter().map(|(k, _)| *k).collect();
            assert_eq!(order, [1, 0]);
        }
        _ => panic!("ждали запись Token"),
    }
    // значение тела — последнее выражение; у тела типа () его нет (решение 4)
    let checked = &func(&p, "checked").body.block;
    assert!(matches!(
        checked.value.as_deref().map(|e| &e.kind),
        Some(ExprKind::If(..))
    ));
    assert_eq!(p.type_text(&checked.ty), "i64");
    let calls = &func(&p, "calls").body.block;
    assert!(calls.value.is_none());
    assert!(matches!(
        calls.stmts.last().map(|st| &st.kind),
        Some(StmtKind::Expr(Expr {
            kind: ExprKind::Call(..),
            ..
        }))
    ));
    // у теста три утверждения на значениях и разбор кортежа
    assert_eq!(p.tests.len(), 3);
    assert!(p.tests.iter().all(|t| t.body.block.ty == Type::Unit));
}

// ───────── тела функций: неправильные программы ─────────

/// Объявления, на которые ссылаются случаи ниже. Номера строк в случаях — от начала случая.
const PRELUDE: &str = "struct P {
\tx: i64
\ty: i64
}
impl Copy for P {}
struct Q {
\tz: i64
}
enum Op {
\tAdd
\tSub
}
enum E {
\tNum(i64)
\tNamed(a: i64, b: f64)
\tEmpty
}
enum Void {}
func forever() -> Never {
\tloop {
\t}
}
func nothing() -> Void {
\tloop {
\t}
}
func two(a: i64, b: i64) -> i64 {
\ta + b
}
func bump(mut n: i64) {
\tn += 1
}
func show(p: P) {
}
func keep(take q: Q) -> Q {
\tq
}
enum W {
\tB(u8)
}
enum Duo {
\tTwo(x: Op, y: Op)
}
enum Half {
\tFull(i64)
\tGone(Never)
\tLost(x: i64, v: Void)
}
struct Nothing {
\tn: Never
}
";

/// Тело с ожидаемой строкой (от начала случая) и сообщением.
const BODY_REJECTED: &[(&str, u32, &str)] = &[
    // затенения нет (12.93)
    (
        "func f(x: i64) {\n\tlet x = 1\n}\n",
        2,
        "имя x уже объявлено в строке 1: затенения нет (12.93)",
    ),
    (
        "func f() {\n\tlet a = 1\n\tif true {\n\t\tlet a = 2\n\t}\n}\n",
        4,
        "имя a уже объявлено в строке 2: затенения нет (12.93)",
    ),
    (
        "func f() {\n\tlet x = 1\n\tlet x = x + 1\n}\n",
        3,
        "имя x уже объявлено в строке 2: затенения нет (12.93)",
    ),
    (
        "func f() {\n\tlet (a, a) = (1, 2)\n}\n",
        2,
        "имя a уже объявлено в строке 2: затенения нет (12.93)",
    ),
    (
        "func f(n: i64) {\n\tvar i = 0\n\twhile i < n {\n\t\tvar n = i\n\t\ti += 1\n\t}\n}\n",
        4,
        "имя n уже объявлено в строке 1: затенения нет (12.93)",
    ),
    // несовпадение типов
    (
        "func f() -> i64 {\n\ttrue\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь bool",
    ),
    (
        "func f(x: u8) -> i64 {\n\tx\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь u8",
    ),
    (
        "func f() {\n\tlet a: u8 = 1\n\tlet b: i64 = 2\n\tlet c = a + b\n}\n",
        4,
        "тип не совпадает: ожидался u8, здесь i64",
    ),
    (
        "func f(a: u8) {\n\tlet c: i64 = a + 1\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь u8",
    ),
    (
        "func f(p: P, q: Q) {\n\tshow(q)\n}\n",
        2,
        "тип не совпадает: ожидался P, здесь Q",
    ),
    (
        "func f() {\n\tif 1 {\n\t}\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(x: i64) {\n\twhile x {\n\t}\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь i64",
    ),
    (
        "func f() {\n\tlet t: (i64, bool) = (1, 2)\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f() -> (i64, i64) {\n\t(1, 2, 3)\n}\n",
        2,
        "тип не совпадает: ожидался (i64, i64), здесь (i64, i64, i64)",
    ),
    (
        "func f() {\n\tlet u: () = two(1, 2)\n}\n",
        2,
        "тип не совпадает: ожидался (), здесь i64",
    ),
    // литералы (12.56, решения 2 и 10а)
    ("func f() {\n\tlet x: u8 = 300\n}\n", 2, "число 300 не помещается в u8 (12.56)"),
    ("func f() {\n\tlet x: i8 = -129\n}\n", 2, "число -129 не помещается в i8 (12.56)"),
    ("func f() {\n\tlet x: u8 = -1\n}\n", 2, "у беззнакового u8 нет смены знака (9.18)"),
    (
        "func f() {\n\tlet x = 99999999999999999999\n}\n",
        2,
        "число 99999999999999999999 не помещается в i64 (12.56)",
    ),
    (
        "func f() {\n\tlet x: f64 = 1\n}\n",
        2,
        "целое число 1 на месте дробного f64: пишут 1.0 (решение 2)",
    ),
    (
        "func f(x: f32) {\n\tlet y = x * 2\n}\n",
        2,
        "целое число 2 на месте дробного f32: пишут 2.0 (решение 2)",
    ),
    (
        "func f() {\n\tlet x: i64 = 1.5\n}\n",
        2,
        "дробное число 1.5 на месте целого i64 (решение 2)",
    ),
    ("func f() {\n\tlet x: f32 = 1e39\n}\n", 2, "число 1e39 не помещается в f32 (12.56)"),
    ("func f() {\n\tlet x = 1e309\n}\n", 2, "число 1e309 не помещается в f64 (12.56)"),
    (
        "func f() {\n\tassert 1 == 1.0\n}\n",
        2,
        "дробное число 1.0 на месте целого i64 (решение 2)",
    ),
    (
        "func f() {\n\tlet x: bool = 1\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f() {\n\tlet x: char = 1.5\n}\n",
        2,
        "тип не совпадает: ожидался char, здесь дробное число",
    ),
    // операции (раздел 4, «Операции»)
    (
        "func f() {\n\tlet x = true + 1\n}\n",
        2,
        "+ — у целых и дробных, а здесь bool",
    ),
    (
        "func f() {\n\tlet x = 'a' + 'b'\n}\n",
        2,
        "+ — у целых и дробных, а здесь char",
    ),
    (
        "func f() {\n\tlet x = true < false\n}\n",
        2,
        "< — у целых, дробных и символов (решение 10д), а здесь bool",
    ),
    (
        "func f(p: P, r: P) {\n\tassert p == r\n}\n",
        2,
        "== — у скаляров (решение 10д), а здесь P",
    ),
    (
        "func f() {\n\tlet x = true & false\n}\n",
        2,
        "& — только у целых (решение 10г), а здесь bool",
    ),
    (
        "func f(y: f64) {\n\tlet x = ~y\n}\n",
        2,
        "~ — только у целых (решение 10г), а здесь f64",
    ),
    (
        "func f(y: u8) {\n\tlet x = -y\n}\n",
        2,
        "смена знака есть только у знаковых целых и дробных (9.18), а здесь u8",
    ),
    (
        "func f() {\n\tlet x = not 1\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(y: i64) {\n\tlet x = y << 1.5\n}\n",
        2,
        "величина сдвига — целое (9.13), а здесь f64",
    ),
    (
        "func f(y: f64) {\n\tlet x = y << 1\n}\n",
        2,
        "<< — только у целых (решение 10г), а здесь f64",
    ),
    (
        "func f(a: bool, b: i64) {\n\tlet x = a and b\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь i64",
    ),
    // преобразования (12.57, 9.16–9.18)
    (
        "func f() {\n\tlet x = bool(1)\n}\n",
        2,
        "i64 не преобразуется в bool (9.17, 9.18)",
    ),
    (
        "func f() {\n\tlet x = i64(true)\n}\n",
        2,
        "bool не преобразуется в i64 (9.17, 9.18)",
    ),
    (
        "func f() {\n\tlet x = f64('a')\n}\n",
        2,
        "char не преобразуется в f64 (9.17, 9.18)",
    ),
    (
        "func f(p: P) {\n\tlet x = u8(p)\n}\n",
        2,
        "в u8 преобразуются только числа и символы, а здесь P",
    ),
    (
        "func f() {\n\tlet x = u8(1, 2)\n}\n",
        2,
        "преобразование u8(…) — один аргумент без имени",
    ),
    // записи и варианты (12.22, 12.35, 12.89, решения 3, 10е, 19)
    (
        "func f() {\n\tlet p = P(1, 2)\n}\n",
        2,
        "запись P создаётся с именами полей: поле: значение (12.35)",
    ),
    ("func f() {\n\tlet p = P(x: 1)\n}\n", 2, "у записи P не задано поле y"),
    (
        "func f() {\n\tlet p = P(x: 1, x: 2, y: 3)\n}\n",
        2,
        "поле x задано дважды",
    ),
    (
        "func f() {\n\tlet p = P(x: 1, z: 2)\n}\n",
        2,
        "у записи P нет поля z",
    ),
    (
        "func f() {\n\tlet p = P\n}\n",
        2,
        "P — тип, а не значение: запись создаётся вызовом P(…)",
    ),
    (
        "func f() {\n\tlet o = Op\n}\n",
        2,
        "Op — тип, а не значение: вариант пишется Op.Вариант",
    ),
    (
        "func f() {\n\tlet o = Op.Mul\n}\n",
        2,
        "у перечисления Op нет варианта Mul",
    ),
    (
        "func f() {\n\tlet o = Op.Add(1)\n}\n",
        2,
        "у варианта Op.Add нет значения",
    ),
    (
        "func f() {\n\tlet e = E.Num\n}\n",
        2,
        "у варианта E.Num есть значение: E.Num(…)",
    ),
    (
        "func f() {\n\tlet e = E.Num(1, 2)\n}\n",
        2,
        "вариант E.Num несёт одно значение без имени: E.Num(значение)",
    ),
    (
        "func f() {\n\tlet e = E.Named(1, 2.0)\n}\n",
        2,
        "вариант E.Named создаётся с именами полей: поле: значение (12.35)",
    ),
    (
        "func f() {\n\tlet e = E.Named(a: 1)\n}\n",
        2,
        "у варианта E.Named не задано поле b",
    ),
    (
        "func f() {\n\tlet e = E.Named(a: 1, b: 2)\n}\n",
        2,
        "целое число 2 на месте дробного f64: пишут 2.0 (решение 2)",
    ),
    (
        "func f() {\n\tlet e = Num(1)\n}\n",
        2,
        "вариант пишется с именем типа: E.Num (12.89)",
    ),
    // вызовы (12.36, 12.59)
    (
        "func f() {\n\tlet x = two(1)\n}\n",
        2,
        "функция two ждёт аргументов: 2, а здесь 1",
    ),
    (
        "func f() {\n\tlet x = two(a: 1, b: 2)\n}\n",
        2,
        "именованных аргументов у функций нет (12.36)",
    ),
    (
        "func f() {\n\tvar n = 1\n\tbump(n)\n}\n",
        3,
        "параметр n функции bump — mut: аргумент пишется с mut (12.59)",
    ),
    (
        "func f() {\n\tvar a = 1\n\tlet x = two(mut a, 2)\n}\n",
        3,
        "параметр a функции two — не mut: mut при вызове не пишется",
    ),
    (
        "func f() {\n\tbump(mut 1)\n}\n",
        2,
        "с mut отдают место: переменную, параметр, их поле или элемент",
    ),
    (
        "func f() {\n\tbump(mut [1])\n}\n",
        2,
        "с mut отдают место: переменную, параметр, их поле или элемент",
    ),
    (
        "func f() {\n\tbump(mut two)\n}\n",
        2,
        "с mut отдают место: переменную, параметр, их поле или элемент",
    ),
    (
        "func f() {\n\tlet n = 1\n\tbump(mut n)\n}\n",
        3,
        "n объявлено через let: его значение не меняется (12.12)",
    ),
    (
        "func f(n: i64) {\n\tbump(mut n)\n}\n",
        2,
        "n — параметр для чтения: его не меняют (12.58)",
    ),
    (
        "func f() {\n\tvar b: u8 = 1\n\tbump(mut b)\n}\n",
        3,
        "тип не совпадает: ожидался i64, здесь u8",
    ),
    (
        "func f() {\n\tlet g = two\n}\n",
        2,
        "не входит в шаг 2: функции как значения",
    ),
    (
        "func f(x: i64) {\n\tx(1)\n}\n",
        2,
        "здесь вызывают значение типа i64, а вызвать можно только функцию",
    ),
    (
        "func f(p: P) {\n\tp.len()\n}\n",
        2,
        "не входит в шаг 2: методы",
    ),
    (
        "func two_of() {\n\tlet t = two(1, 2)(3)\n}\n",
        2,
        "здесь вызывают значение типа i64, а вызвать можно только функцию",
    ),
    // поля и элементы (12.19)
    ("func f(p: P) {\n\tlet z = p.z\n}\n", 2, "у записи P нет поля z"),
    ("func f(x: i64) {\n\tlet z = x.f\n}\n", 2, "у типа i64 нет полей"),
    (
        "func f(e: E) {\n\tlet z = e.a\n}\n",
        2,
        "значение варианта берут образцом: у перечисления E нет полей",
    ),
    (
        "func f(x: i64) {\n\tlet z = x[0]\n}\n",
        2,
        "номер берут только у массива, а здесь i64",
    ),
    (
        "func f(a: Array[i64, 2], i: u8) {\n\tlet z = a[i]\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь u8",
    ),
    (
        "func f(a: Array[i64, 2]) {\n\tlet z = a[1.5]\n}\n",
        2,
        "дробное число 1.5 на месте целого i64 (решение 2)",
    ),
    (
        "func f(a: Array[i64, 2]) {\n\tlet z = a[0, 1]\n}\n",
        2,
        "у элемента массива один номер: a[i]",
    ),
    (
        "func f(a: Array[i64, 2]) {\n\tlet z = a[any]\n}\n",
        2,
        "не входит в шаг 2: any",
    ),
    (
        "func f(a: Array[i64, 2]) {\n\tlet z = a[uses E]\n}\n",
        2,
        "не входит в шаг 2: uses",
    ),
    (
        "func f() {\n\tlet p = P[i64](x: 1, y: 2)\n}\n",
        2,
        "не входит в шаг 2: обобщения",
    ),
    (
        "func f() {\n\tlet x = two[i64](1, 2)\n}\n",
        2,
        "не входит в шаг 2: обобщения",
    ),
    (
        "func f() {\n\tlet z = P(x: 1, y: 2).z\n}\n",
        2,
        "у записи P нет поля z",
    ),
    // массивы (12.37)
    (
        "func f() {\n\tlet a = [1, 2]\n}\n",
        2,
        "у массива не указан тип: без типа [ … ] — список, а его в шаге 2 нет (12.37); укажите Array[T, N]",
    ),
    (
        "func f() {\n\tlet a: Array[i64, 3] = [1, 2]\n}\n",
        2,
        "в массиве Array[i64, 3] должно быть элементов: 3, а здесь 2",
    ),
    (
        "func f() {\n\tlet a: i64 = [1]\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь массив",
    ),
    // запись в место (12.12, 12.54, 12.58)
    (
        "func f() {\n\tlet x = 1\n\tx = 2\n}\n",
        3,
        "x объявлено через let: его значение не меняется (12.12)",
    ),
    (
        "func f(x: i64) {\n\tx = 1\n}\n",
        2,
        "x — параметр для чтения: его не меняют (12.58)",
    ),
    (
        "func f() {\n\tlet p = P(x: 1, y: 2)\n\tp.x = 3\n}\n",
        3,
        "p объявлено через let: его значение не меняется (12.12)",
    ),
    (
        "func f() {\n\ttwo(1, 2) = 3\n}\n",
        2,
        "присваивают месту: переменной, параметру, их полю или элементу",
    ),
    (
        "func f() {\n\ttwo = 3\n}\n",
        2,
        "присваивают месту: переменной, параметру, их полю или элементу",
    ),
    (
        "func f() {\n\tP(x: 1, y: 2).x = 3\n}\n",
        2,
        "присваивают месту: переменной, параметру, их полю или элементу",
    ),
    (
        "func f() {\n\tvar x = 1\n\tx += 1.5\n}\n",
        3,
        "дробное число 1.5 на месте целого i64 (решение 2)",
    ),
    (
        "func f() {\n\tvar b = true\n\tb += true\n}\n",
        3,
        "+ — у целых и дробных, а здесь bool",
    ),
    (
        "func f() {\n\tvar b = true\n\tb -= 1\n}\n",
        3,
        "- — у целых и дробных, а здесь bool",
    ),
    (
        "func f() {\n\tvar x = 1.5\n\tx |= 1\n}\n",
        3,
        "| — только у целых (решение 10г), а здесь f64",
    ),
    (
        "func f() {\n\tvar x = 1.5\n\tx >>= 1\n}\n",
        3,
        ">> — только у целых (решение 10г), а здесь f64",
    ),
    (
        "func f() {\n\tvar x = 1\n\tx <<= 1.5\n}\n",
        3,
        "величина сдвига — целое (9.13), а здесь f64",
    ),
    (
        "func f() {\n\tvar x = 1\n\tx = true\n}\n",
        3,
        "тип не совпадает: ожидался i64, здесь bool",
    ),
    // циклы, метки, break и continue (12.45, решения 10ж и 12)
    ("func f() {\n\tbreak\n}\n", 2, "break стоит вне цикла"),
    ("func f() {\n\tcontinue\n}\n", 2, "continue стоит вне цикла"),
    (
        "func f() {\n\tloop {\n\t\tbreak outer\n\t}\n}\n",
        3,
        "break: нет цикла с меткой outer",
    ),
    (
        "func f() {\n\ta: loop {\n\t\ta: loop {\n\t\t\tbreak a\n\t\t}\n\t}\n}\n",
        3,
        "метка a уже у внешнего цикла в строке 2 (решение 10ж)",
    ),
    (
        "func f() {\n\twhile true {\n\t} end loop\n}\n",
        2,
        "метка end loop не совпадает с началом цикла while",
    ),
    (
        "func f() {\n\tbusy: while true {\n\t} end idle\n}\n",
        2,
        "метка end idle не совпадает с меткой цикла busy",
    ),
    (
        "func f() {\n\tbusy: loop {\n\t\tbreak\n\t} end loop\n}\n",
        2,
        "метка end loop не совпадает с меткой цикла busy",
    ),
    (
        "func f() {\n\tloop {\n\t\tbreak\n\t} end while\n}\n",
        2,
        "метка end while не совпадает с началом цикла loop",
    ),
    (
        "func f() {\n\tfor x in y {\n\t}\n}\n",
        2,
        "не входит в шаг 2: for",
    ),
    // return (12.15)
    (
        "func f() -> i64 {\n\treturn\n}\n",
        2,
        "return без значения, а результат функции — i64",
    ),
    (
        "func f() {\n\treturn 1\n}\n",
        2,
        "тип не совпадает: ожидался (), здесь целое число",
    ),
    (
        "func f() -> Never {\n\treturn\n}\n",
        2,
        "return без значения, а результат функции — Never",
    ),
    // значение тела и блока (12.15, решение 4)
    (
        "func f() -> i64 {\n\tlet x = 1\n}\n",
        2,
        "блок должен дать значение типа i64, а его последняя инструкция значения не даёт",
    ),
    ("func f() -> i64 {\n}\n", 1, "блок пуст, а должен дать значение типа i64"),
    (
        "func f() {\n\t1\n}\n",
        2,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(w: f64, h: f64) {\n\tw * h\n}\n",
        2,
        "значение типа f64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(x: i64) -> i64 {\n\tx + 1\n\tx\n}\n",
        2,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(c: bool) {\n\tif c {\n\t\t1\n\t} else {\n\t\t2\n\t}\n}\n",
        3,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(c: bool) {\n\tlet x: i64 = if c { 1 }\n}\n",
        2,
        "if без else даёт (), а ожидался i64",
    ),
    (
        "func f(c: bool) -> i64 {\n\tif c { 1 } else { true }\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь bool",
    ),
    (
        "func f(c: bool, x: u8) {\n\tlet v = if c { x } else { true }\n}\n",
        2,
        "тип не совпадает: ожидался u8, здесь bool",
    ),
    // недостижимое (решение 7)
    (
        "func f() -> i64 {\n\treturn 1\n\tlet x = 2\n}\n",
        3,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    (
        "func f() {\n\tloop {\n\t}\n\tlet x = 1\n}\n",
        4,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    (
        "func f() {\n\tloop {\n\t\tbreak\n\t\tlet x = 1\n\t}\n}\n",
        4,
        "сюда выполнение не дойдёт: инструкция в строке 3 всегда уходит (решение 7)",
    ),
    (
        "func f(c: bool) -> i64 {\n\tif c {\n\t\treturn 1\n\t} else {\n\t\treturn 2\n\t}\n\t3\n}\n",
        7,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    (
        "func f() {\n\tforever()\n\tlet x = 1\n}\n",
        3,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    (
        "func f() {\n\tnothing()\n\tlet x = 1\n}\n",
        3,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    // где стоит значение типа без значений (решение 14)
    (
        "func f() {\n\tlet x: i64 = forever()\n}\n",
        2,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f() {\n\tlet x = two(1, forever())\n}\n",
        2,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f() -> i64 {\n\treturn nothing()\n}\n",
        2,
        "выражение типа Void значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f() {\n\tif forever() {\n\t}\n}\n",
        2,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f() -> i64 {\n\tforever().x\n}\n",
        2,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f() {\n\tvar x = 1\n\tx = nothing()\n}\n",
        3,
        "выражение типа Void значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f() -> Never {\n\tlet b = forever() == forever()\n}\n",
        2,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f(c: bool) -> Never {\n\tif c {\n\t\tforever()\n\t}\n}\n",
        2,
        "if без else даёт (), а ожидался Never",
    ),
    // assert (12.48, решения 10к и 20)
    (
        "func f() {\n\tassert 1\n}\n",
        2,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(c: bool) {\n\tassert c, 1\n}\n",
        2,
        "сообщение assert — строка: assert условие, \"пояснение\" (12.48)",
    ),
    (
        "func f(c: bool, y: i64) {\n\tassert c, \"a $y\"\n}\n",
        2,
        "сообщение assert — строка без подстановок (решение 10к)",
    ),
    (
        "func f(c: bool) {\n\tassert c, \"\"\"a\"\"\"\n}\n",
        2,
        "не входит в шаг 2: многострочная строка в сообщении assert",
    ),
    // let с образцом (решение 18)
    (
        "func f(t: (i64, i64)) {\n\tlet (a, 1) = t\n}\n",
        2,
        "в let образец подходит всегда: только имена, _ и кортежи (решение 18)",
    ),
    (
        "func f() {\n\tlet (a, b) = 1\n}\n",
        2,
        "образец из 2 частей, а значение типа i64",
    ),
    (
        "func f() {\n\tlet (a, b, c) = (1, 2)\n}\n",
        2,
        "образец из 3 частей, а значение типа (i64, i64)",
    ),
    (
        "func f(o: Op) {\n\tlet (Add, b) = (o, 1)\n}\n",
        2,
        "в let образец подходит всегда, а Add — вариант Op (решение 18)",
    ),
    // имена
    ("func f() {\n\tlet x = y\n}\n", 2, "нет имени y"),
    ("func f() {\n\tlet x = i64\n}\n", 2, "i64 — тип, а не значение"),
    ("func f() {\n\tlet x = Never\n}\n", 2, "Never — тип, а не значение"),
    // конструкции вне шага 2 в телах (раздел 3)
    ("func f() {\n\tlet s = \"a\"\n}\n", 2, "не входит в шаг 2: строки"),
    ("func f() {\n\tconst c = 1\n}\n", 2, "не входит в шаг 2: const"),
    (
        "func f(o: Op) {\n\tlet b = o is Op.Add\n}\n",
        2,
        "не входит в шаг 2: is",
    ),
    (
        "func f() {\n\tlet g = func(x: i64) -> i64 { x }\n}\n",
        2,
        "не входит в шаг 2: замыкания",
    ),
    ("func f() {\n\tlet x = Some(1)\n}\n", 2, "не входит в шаг 2: Option"),
    ("func f() {\n\tlet x = Ok(1)\n}\n", 2, "не входит в шаг 2: Result"),
    ("func f() {\n\tprint(1)\n}\n", 2, "не входит в шаг 2: print"),
    (
        "func f() {\n\tlet r = 0..<3\n}\n",
        2,
        "не входит в шаг 2: диапазоны ..< и ..=",
    ),
    ("func f() {\n\tdiscard self\n}\n", 2, "не входит в шаг 2: методы и self"),
    ("func f() {\n\tlet s = self\n}\n", 2, "не входит в шаг 2: методы и self"),
    (
        "func f() {\n\tlet x = two(1, 2) catch {\n\t\t0\n\t}\n}\n",
        2,
        "не входит в шаг 2: catch",
    ),
    ("func f() {\n\tlet x = try two(1, 2)\n}\n", 2, "не входит в шаг 2: try"),
    ("func f() {\n\tlet x = await two(1, 2)\n}\n", 2, "не входит в шаг 2: await"),
    (
        "func f() {\n\ttrust \"проверено\" {\n\t}\n}\n",
        2,
        "не входит в шаг 2: trust",
    ),
    (
        "func f() {\n\tprovide a = 1 {\n\t}\n}\n",
        2,
        "не входит в шаг 2: provide",
    ),
    (
        "func f() {\n\ttarget {\n\t\telse {\n\t\t}\n\t}\n}\n",
        2,
        "не входит в шаг 2: target",
    ),
    (
        "func f() {\n\t@prototype {\n\t\ttwo(1, 2)\n\t}\n}\n",
        2,
        "не входит в шаг 2: блоки с пометкой уровня или строгости",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t}\n}\n",
        2,
        "match без ветвей бывает только по значению типа без значений, а у Op значения есть (решение 28); добавьте ветвь или else",
    ),
    // символ и строка сообщения — скалярные значения Unicode (9.8)
    (
        "func f() {\n\tlet c = '\\u{D800}'\n}\n",
        2,
        "\\u{D800} — не скалярное значение Unicode",
    ),
    (
        "func f(c: bool) {\n\tassert c, \"\\u{110000}\"\n}\n",
        2,
        "\\u{110000} — не скалярное значение Unicode",
    ),
    // тип без значений: как Never ведёт себя выражение типа Never и вызов функции с результатом
    // — перечислением без вариантов (9.36, решение 10в); другое значение типа Void — обычное
    (
        "func f(v: Void) -> i64 {\n\tv\n}\n",
        2,
        "тип не совпадает: ожидался i64, здесь Void",
    ),
    (
        "func f(v: Void) {\n\tv\n\tlet x = 1\n}\n",
        2,
        "значение типа Void не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(n: Never) {\n\tn\n\tlet x = 1\n}\n",
        3,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    // условие while — не тело цикла
    (
        "func f(c: bool) {\n\twhile if c { break } else { true } {\n\t}\n}\n",
        2,
        "break стоит в условии while, а условие — не тело цикла (решение 23): перенесите проверку в тело",
    ),
    (
        "func f(c: bool) {\n\touter: loop {\n\t\twhile if c { continue outer } else { true } {\n\t\t}\n\t}\n}\n",
        3,
        "continue стоит в условии while, а условие — не тело цикла (решение 23): перенесите проверку в тело",
    ),
    // имя, которого нет, в месте записи
    ("func f() {\n\ty = 1\n}\n", 2, "нет имени y"),
    ("func f() {\n\tbump(mut zz)\n}\n", 2, "нет имени zz"),
    ("func f() {\n\tzz.a[0] += 1\n}\n", 2, "нет имени zz"),
    // место, на котором операции нет, тип операндам не даёт
    (
        "func f(a: i64, b: i64) {\n\tlet x: f64 = a & b\n}\n",
        2,
        "тип не совпадает: ожидался f64, здесь i64",
    ),
    (
        "func f(a: bool, b: bool) {\n\tlet x: bool = a + b\n}\n",
        2,
        "+ — у целых и дробных, а здесь bool",
    ),
    // совет — десятичная запись
    (
        "func f() {\n\tlet x: f64 = 0x10\n}\n",
        2,
        "целое число 0x10 на месте дробного f64: пишут 16.0 (решение 2)",
    ),
    (
        "func f() {\n\tlet x: f32 = -0b101\n}\n",
        2,
        "целое число -0b101 на месте дробного f32: пишут -5.0 (решение 2)",
    ),
    (
        "func f() {\n\tlet x: f64 = 1_000\n}\n",
        2,
        "целое число 1_000 на месте дробного f64: пишут 1000.0 (решение 2)",
    ),
    // значение if без else
    (
        "func f(c: bool) {\n\tlet v = if c {\n\t\t1\n\t}\n}\n",
        3,
        "if без else даёт (), и значение типа i64 здесь пропадёт (решение 4): допишите else",
    ),
    (
        "func f(c: bool) {\n\tlet v = if c {\n\t\t1\n\t\ttwo(1, 2)\n\t}\n}\n",
        3,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(c: bool) {\n\tif c {\n\t\t1\n\t}\n}\n",
        3,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(c: bool, d: bool) {\n\tlet v = if c {\n\t\twhile d {\n\t\t\t1\n\t\t}\n\t}\n}\n",
        4,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    // блоки типа ()
    (
        "test \"t\" {\n\t1\n}\n",
        2,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(c: bool) {\n\twhile c {\n\t\t1\n\t}\n}\n",
        3,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f() {\n\tloop {\n\t\t1\n\t}\n}\n",
        3,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(c: bool) {\n\t(c)\n}\n",
        2,
        "значение типа bool не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f() -> i64 {\n\twhile true {\n\t}\n}\n",
        2,
        "блок должен дать значение типа i64, а его последняя инструкция значения не даёт",
    ),
    // недостижимое и метки
    (
        "func f() {\n\tloop {\n\t\tcontinue\n\t\tlet x = 1\n\t}\n}\n",
        4,
        "сюда выполнение не дойдёт: инструкция в строке 3 всегда уходит (решение 7)",
    ),
    (
        "func f() {\n\ta: loop {\n\t\tloop {\n\t\t\ta: loop {\n\t\t\t\tbreak a\n\t\t\t}\n\t\t\tbreak\n\t\t}\n\t\tbreak\n\t}\n}\n",
        4,
        "метка a уже у внешнего цикла в строке 2 (решение 10ж)",
    ),
    // литералы и операции
    (
        "func f() {\n\tlet x: u8 = 300 + 1\n}\n",
        2,
        "число 300 не помещается в u8 (12.56)",
    ),
    (
        "func f() {\n\tlet w = W.B(300)\n}\n",
        2,
        "число 300 не помещается в u8 (12.56)",
    ),
    (
        "func f() {\n\tlet x = () == ()\n}\n",
        2,
        "== — у скаляров (решение 10д), а здесь ()",
    ),
    (
        "func f() {\n\tlet x = (1, 2) == (1, 2)\n}\n",
        2,
        "== — у скаляров (решение 10д), а здесь (i64, i64)",
    ),
    // скобки отделяют число от минуса (решение 25)
    (
        "func f() {\n\tlet x: i8 = -(128)\n}\n",
        2,
        "число 128 не помещается в i8 (12.56)",
    ),
    // место на сравнение не влияет: результат — bool
    (
        "func f(x: i64) {\n\tlet n: u8 = 1 < x\n}\n",
        2,
        "тип не совпадает: ожидался u8, здесь bool",
    ),
    (
        "func f() {\n\tlet x = char(1.5)\n}\n",
        2,
        "f64 не преобразуется в char (9.17, 9.18)",
    ),
    (
        "func f(q: Q) {\n\tlet r = keep(mut q)\n}\n",
        2,
        "параметр q функции keep — не mut: mut при вызове не пишется",
    ),
    // стандартные имена значениями не бывают
    (
        "func f() {\n\tlet x = Copy\n}\n",
        2,
        "Copy — интерфейс, а не значение",
    ),
    (
        "func f() {\n\tlet x = Add\n}\n",
        2,
        "вариант пишется с именем типа: Op.Add (12.89)",
    ),
    ("func f() {\n\tlet x = size_of\n}\n", 2, "не входит в шаг 2: size_of"),
];

#[test]
fn wrong_bodies_are_rejected() {
    let prelude_lines = PRELUDE.lines().count() as u32;
    for (body, line, message) in BODY_REJECTED {
        let text = format!("{}{}", PRELUDE, body);
        let message = shift_lines(message, prelude_lines);
        rejected(&text, prelude_lines + line, &message);
    }
}

/// Сдвигает номера после «строке » в сообщении: в случаях они от начала случая.
fn shift_lines(message: &str, by: u32) -> String {
    const MARK: &str = "строке ";
    let mut out = String::new();
    let mut rest = message;
    while let Some(at) = rest.find(MARK) {
        let (head, tail) = rest.split_at(at + MARK.len());
        out.push_str(head);
        let digits = tail.chars().take_while(char::is_ascii_digit).count();
        match tail[..digits].parse::<u32>() {
            Ok(n) => out.push_str(&(n + by).to_string()),
            Err(_) => panic!("после «строке» нет номера: {}", message),
        }
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}

#[test]
fn prelude_of_wrong_bodies_is_accepted() {
    accepted(PRELUDE);
}

#[test]
fn local_names_may_hide_package_names_and_repeat_in_sibling_blocks() {
    // 12.93: локальное имя может скрыть имя пакета; в соседних блоках имена повторяются
    let p = accepted(
        "func g() -> i64 {\n\t1\n}\nfunc f(c: bool) -> i64 {\n\tif c {\n\t\tlet a = 1\n\t} else {\n\t\tlet a = 2\n\t}\n\tlet a = 3\n\tlet g = 4\n\ta + g\n}\n",
    );
    let names: Vec<&str> = func(&p, "f")
        .body
        .locals
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, ["c", "a", "a", "a", "g"]);
}

#[test]
fn local_names_may_hide_builtin_type_names() {
    // 12.93 и раздел 4: в выражении имя ведёт к локальному, в месте типа — к типу
    let p = accepted("func f() -> i64 {\n\tlet i64: i64 = 4\n\tlet Never = i64 + 1\n\tNever\n}\n");
    // стандартные имена локальное имя тоже скрывает (решение 26)
    let q = accepted("func g(Option: i64) -> i64 {\n\tlet print = 1\n\tvar Copy = 2\n\tCopy += print\n\tlet Some = Copy + Option\n\tSome\n}\n");
    assert_eq!(
        local_types(&q, "g"),
        pairs(&[
            ("Option", "i64"),
            ("print", "i64"),
            ("Copy", "i64"),
            ("Some", "i64")
        ])
    );
    // пробел между минусом и числом литерал не разрывает (решение 25)
    let r = accepted("func h() -> i8 {\n\t- 128\n}\n");
    assert_eq!(
        func(&r, "h").body.block.value.as_deref().map(|e| &e.kind),
        Some(&ExprKind::Int(-128))
    );
    assert_eq!(
        local_types(&p, "f"),
        pairs(&[("i64", "i64"), ("Never", "i64")])
    );
    rejected(
        "func f() {\n\tlet i64 = 4\n\tlet x = i64(1)\n}\n",
        3,
        "здесь вызывают значение типа i64, а вызвать можно только функцию",
    );
}

#[test]
fn bodies_are_checked_in_file_order() {
    // тело теста выше тела функции: первой называется ошибка в тесте
    let text = "test \"t\" {\n\tlet x = y\n}\nfunc f() {\n\tlet z = w\n}\n";
    rejected(text, 2, "нет имени y");
    let text = "func f() {\n\tlet z = w\n}\ntest \"t\" {\n\tlet x = y\n}\n";
    rejected(text, 2, "нет имени w");
}

#[test]
fn long_chains_stay_flat() {
    // цепочка операций не вкладывается сама в себя (13.5): проверка идёт на маленьком стеке,
    // а дерево удаляется на стеке теста
    let n = 100_000;
    let mut text = String::from("func f(x: i64, c: bool) -> bool {\n\tlet s = x");
    for _ in 1..n {
        text.push_str(" + 1");
    }
    text.push_str("\n\tc");
    for _ in 1..n {
        text.push_str(" and c");
    }
    text.push_str("\n}\n");
    let p = match run_on_small_stack(text) {
        Ok(p) => p,
        Err((line, message)) => panic!("отвергнуто, строка {}: {}", line, message),
    };
    let body = &func(&p, "f").body.block;
    match &body.stmts[0].kind {
        StmtKind::Let(
            _,
            Expr {
                kind: ExprKind::Binary(_, rest),
                ..
            },
        ) => assert_eq!(rest.len(), n - 1),
        _ => panic!("ждали цепочку сложений"),
    }
    match body.value.as_deref().map(|e| &e.kind) {
        Some(ExprKind::And(parts)) => assert_eq!(parts.len(), n),
        _ => panic!("ждали цепочку and"),
    }
}

#[test]
fn literals_take_their_type_from_the_place_or_an_operand() {
    // раздел 4, «Операции»: тип от места, иначе от операнда или ветви со своим типом
    let p = accepted(&format!(
        "{}{}",
        PRELUDE,
        "func f(x: u8, y: i8, m: u8, b: u8, c: bool, t: (u8, u8), a: Array[u8, 2], z: Array[u8, 0], tb: (u8, bool)) {
\tlet k1 = 2 * x
\tlet k2 = -1 * y
\tlet k3 = x + (1 + 2)
\tlet k4 = x + if c { 1 } else { 2 }
\tlet k5 = if c { 1 } else { 2 } + x
\tlet k6: u8 = 1 << 3
\tlet k7 = 1.5 + 2.5
\tlet k8: u8 = ~0
\tlet k9: i8 = -(1)
\tlet k10 = if c { [1, 2] } else { a }
\tlet k11 = if c { (1, 2) } else { t }
\tlet k12 = if c { [] } else { z }
\tlet k13 = m & (1 << b)
\tlet k14 = (1 << b) & m
\tlet k15 = W.B(200)
\tlet k16 = c == true
\tlet k17 = 'a' < 'b'
\tlet k18 = 1 << b
\tlet k19 = if c { (1, true) } else { tb }
\tlet k20 = (1 + 2) * x
}
"
    ));
    let types = local_types(&p, "f");
    let want = [
        ("k1", "u8"),
        ("k2", "i8"),
        ("k3", "u8"),
        ("k4", "u8"),
        ("k5", "u8"),
        ("k6", "u8"),
        ("k7", "f64"),
        ("k8", "u8"),
        ("k9", "i8"),
        ("k10", "Array[u8, 2]"),
        ("k11", "(u8, u8)"),
        ("k12", "Array[u8, 0]"),
        ("k13", "u8"),
        ("k14", "u8"),
        ("k15", "W"),
        ("k16", "bool"),
        ("k17", "bool"),
        ("k18", "i64"),
        ("k19", "(u8, bool)"),
        ("k20", "u8"),
    ];
    assert_eq!(types[9..], pairs(&want)[..]);
}

#[test]
fn loops_and_labels_are_accepted() {
    // break без метки — у ближайшего цикла: внешний loop остаётся без break и уходит (9.36)
    let p = accepted("func f() -> i64 {\n\tloop {\n\t\twhile true {\n\t\t\tbreak\n\t\t}\n\t}\n}\n");
    assert_eq!(func(&p, "f").body.block.ty, Type::Never);
    // break с меткой — у цикла с меткой: после него код достижим
    accepted("func f() {\n\ta: loop {\n\t\tbreak a\n\t}\n\tlet x = 1\n}\n");
    // соседние циклы с одной меткой; while true имеет тип () (раздел 4)
    accepted("func f() {\n\ta: loop {\n\t\tbreak a\n\t} end a\n\ta: while true {\n\t} end a\n\tlet x = 1\n}\n");
    // цикл внутри условия while: break в нём относится к нему
    accepted("func f(c: bool) {\n\twhile if c {\n\t\tloop {\n\t\t\tbreak\n\t\t}\n\t\ttrue\n\t} else {\n\t\tfalse\n\t} {\n\t\tbreak\n\t}\n}\n");
    // if в скобках, стоящий инструкцией, — тоже инструкция (решение 4)
    accepted(&format!(
        "{}func g(c: bool) {{\n\t(if c {{ two(1, 2) }} else {{ two(3, 4) }})\n\tkeep(Q(z: 1))\n}}\n",
        PRELUDE
    ));
}

// ───────── match: неправильные программы ─────────

/// Случаи `match` на объявлениях из `PRELUDE`; строки — от начала случая.
const MATCH_REJECTED: &[(&str, u32, &str)] = &[
    // полнота (9.35, раздел 6)
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, Op.Sub не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(_) {}\n\t\tcase Empty {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, E.Named(..) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(1) {}\n\t\tcase Named(..) {}\n\t\tcase Empty {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, E.Num(0) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(t: (bool, bool)) {\n\tmatch t {\n\t\tcase (_, true) {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, (_, false) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(t: (bool, bool)) {\n\tmatch t {\n\t\tcase (true, _) {}\n\t\tcase (false, true) {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, (false, false) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(o: Op, c: bool) {\n\tmatch o {\n\t\tcase Add if c {}\n\t\tcase Sub {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, Op.Add не подходит ни к одной ветви без условия, а ветви с условием в полноте не участвуют (9.35); добавьте ветвь или else",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 0 {}\n\t\tcase 1 {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, 2 не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    // пример — из значений, к которым не подходит и ветвь с условием
    (
        "func f(o: Op, c: bool) {\n\tmatch o {\n\t\tcase Add if c {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, Op.Sub не подходит ни к одной ветви без условия, а ветви с условием в полноте не участвуют (9.35); добавьте ветвь или else",
    ),
    // ветвей без условия нет: пример не нужен
    (
        "func f(x: i8) {\n\tmatch x {\n\t\tcase n if n > 0 {}\n\t}\n}\n",
        2,
        "у match нет ни одной ветви без условия, а ветви с условием в полноте не участвуют (9.35); добавьте ветвь без условия или else",
    ),
    (
        "func f(c: char) {\n\tmatch c {\n\t\tcase 'a' {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, 'b' не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    // match без ветвей — только по типу без значений (решение 28)
    (
        "func f(b: bool) {\n\tmatch b {\n\t}\n}\n",
        2,
        "match без ветвей бывает только по значению типа без значений, а у bool значения есть (решение 28); добавьте ветвь или else",
    ),
    (
        "func f(p: P) {\n\tmatch p {\n\t}\n}\n",
        2,
        "match без ветвей бывает только по значению типа без значений, а у P значения есть (решение 28); добавьте ветвь или else",
    ),
    (
        "func f(u: ()) {\n\tmatch u {\n\t}\n}\n",
        2,
        "match без ветвей бывает только по значению типа без значений, а у () значения есть (решение 28); добавьте ветвь или else",
    ),
    // пример с полями: до последнего разобранного — по именам, остальные — `..`
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(a: 1, ..) {}\n\t\tcase Num(_) {}\n\t\tcase Empty {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, E.Named(a: 0, ..) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(d: Duo) {\n\tmatch d {\n\t\tcase Two(_, y: Add) {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, Duo.Two(_, y: Op.Sub) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(d: Duo) {\n\tmatch d {\n\t\tcase Two(x: Add, y: Add) {}\n\t\tcase Two(x: Sub, ..) {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, Duo.Two(x: Op.Add, y: Op.Sub) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(w: W) {\n\tmatch w {\n\t\tcase B(0) {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, W.B(1) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Empty {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, E.Num(_) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    // в примере сначала неразобранный вариант, а не неразобранная часть разобранного
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(1) {}\n\t\tcase Empty {}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, E.Named(..) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    // недостижимое (решение 7)
    // ветвь выше, которая одна разобрала всё, названа строкой
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {}\n\t\tcase Sub {}\n\t\tcase Add {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add, Add {}\n\t\tcase Sub {}\n\t}\n}\n",
        3,
        "альтернатива 2 недостижима: все её значения разобраны альтернативой 1 (решение 7)",
    ),
    (
        "func f(o: Op, c: bool) {\n\tmatch o {\n\t\tcase Add {}\n\t\tcase Add, Sub if c {}\n\t\telse {}\n\t}\n}\n",
        4,
        "альтернатива 1 недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    // несколько ветвей вместе — без строки
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {}\n\t\tcase Sub {}\n\t\tcase Add, Sub {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвями выше (решение 7)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 1 {}\n\t\tcase 2 {}\n\t\tcase 3, 1, 2 {}\n\t\telse {}\n\t}\n}\n",
        5,
        "альтернатива 2 недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(t: (bool, bool)) {\n\tmatch t {\n\t\tcase (true, _) {}\n\t\tcase (false, _) {}\n\t\tcase (_, true), (false, false) {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвями выше (решение 7)",
    ),
    (
        "func f(t: (bool, bool), c: bool) {\n\tmatch t {\n\t\tcase (true, _) {}\n\t\tcase (false, true) {}\n\t\tcase (_, false), (_, true) if c {}\n\t\telse {}\n\t}\n}\n",
        5,
        "альтернатива 2 недостижима: все её значения разобраны выше (решение 7)",
    ),
    // названа первая ветвь без условия, которая одна разбирает все значения: ветвь выше с
    // другим значением или с условием не подходит
    (
        "func f(o: Op, c: bool) {\n\tmatch o {\n\t\tcase Add if c {}\n\t\tcase Add {}\n\t\tcase Add {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 2 {}\n\t\tcase 1 {}\n\t\tcase 1 {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    (
        "func f(c: char) {\n\tmatch c {\n\t\tcase 'b' {}\n\t\tcase 'a' {}\n\t\tcase 'a' {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    (
        "func f(b: bool) {\n\tmatch b {\n\t\tcase false {}\n\t\tcase true {}\n\t\tcase true {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Empty {}\n\t\tcase Num(1) {}\n\t\tcase Num(1) {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    (
        "func f(t: (bool, i64)) {\n\tmatch t {\n\t\tcase (false, 1) {}\n\t\tcase (true, 1) {}\n\t\tcase (true, 1) {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(2) {}\n\t\tcase Num(1) {}\n\t\tcase Num(1) {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 4 (решение 7)",
    ),
    // строка альтернативы — её собственная
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {}\n\t\tcase Sub,\n\t\t\tAdd {}\n\t}\n}\n",
        5,
        "альтернатива 2 недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    // имя, которое разобрало всё, у записи — просто имя: вариантов у неё нет
    (
        "func f(p: P) {\n\tmatch p {\n\t\tcase x {}\n\t\telse {}\n\t}\n}\n",
        4,
        "else недостижим: ветвь в строке 3 подходит к любому значению (решение 7)",
    ),
    // одна ветвь выше разобрала все альтернативы
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(_) {}\n\t\tcase Num(1), Num(2) {}\n\t\telse {}\n\t}\n}\n",
        4,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {}\n\t\tcase Sub {}\n\t\telse {}\n\t}\n}\n",
        5,
        "else недостижим: все значения разобраны ветвями выше (решение 7)",
    ),
    // ветвь выше подходит к любому значению
    (
        "func f(o: Op) {\n\tmatch o {\n\t\telse {}\n\t\tcase Add {}\n\t}\n}\n",
        4,
        "ветвь недостижима: else в строке 3 подходит к любому значению (решение 7); удалите эту ветвь или поставьте её выше",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase _ {}\n\t\telse {}\n\t}\n}\n",
        4,
        "else недостижим: ветвь в строке 3 подходит к любому значению (решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\telse {}\n\t\telse {}\n\t}\n}\n",
        4,
        "else недостижим: else в строке 3 подходит к любому значению (решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase _, Add {}\n\t}\n}\n",
        3,
        "альтернатива 2 недостижима: альтернатива 1 подходит к любому значению (решение 7)",
    ),
    // имя, которое не вариант, — новое имя: опечатка в варианте (12.42)
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Ad {}\n\t\tcase Sub {}\n\t}\n}\n",
        4,
        "ветвь недостижима: ветвь в строке 3 подходит к любому значению — у Op нет варианта Ad, поэтому Ad там новое имя (12.42, решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Ad, Ad {}\n\t}\n}\n",
        3,
        "альтернатива 2 недостижима: альтернатива 1 подходит к любому значению — у Op нет варианта Ad, поэтому Ad там новое имя (12.42, решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase x {}\n\t\telse {}\n\t}\n}\n",
        4,
        "else недостижим: ветвь в строке 3 подходит к любому значению — у Op нет варианта x, поэтому x там новое имя (12.42, решение 7)",
    ),
    // строка else — строка слова else
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {}\n\t\tcase Sub {}\n\t\telse\n\t\t{}\n\t}\n}\n",
        5,
        "else недостижим: все значения разобраны ветвями выше (решение 7)",
    ),

    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase n {}\n\t\tcase 0 {}\n\t}\n}\n",
        4,
        "ветвь недостижима: ветвь в строке 3 подходит к любому значению (решение 7); удалите эту ветвь или поставьте её выше",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 16 {}\n\t\tcase 0x10 {}\n\t\telse {}\n\t}\n}\n",
        4,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(_) {}\n\t\tcase Num(1) {}\n\t\telse {}\n\t}\n}\n",
        4,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(t: (bool, i64)) {\n\tmatch t {\n\t\tcase (true, 1) {}\n\t\tcase (b, _) {}\n\t\tcase (true, 1) {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(c: char, b: bool) {\n\tmatch (c, b) {\n\t\tcase ('a', true) {}\n\t\tcase ('a', false) {}\n\t\tcase ('a', true) {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(d: Duo) {\n\tmatch d {\n\t\tcase Two(x: Add, y: _) {}\n\t\tcase Two(x: Add, y: Sub) {}\n\t\telse {}\n\t}\n}\n",
        4,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    // к ветви, где у части типа нет значений, не подходит ни одно значение (решение 28)
    (
        "func f(h: Half) {\n\tmatch h {\n\t\tcase Full(_) {}\n\t\tcase Gone(_) {}\n\t}\n}\n",
        4,
        "к ветви не подходит ни одно значение: у типа Never значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(h: Half) {\n\tmatch h {\n\t\tcase Lost(x: 1, ..) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Void значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(h: Half) {\n\tmatch h {\n\t\tcase Full(_), Gone(_) {}\n\t}\n}\n",
        3,
        "к альтернативе 2 не подходит ни одно значение: у типа Never значений нет (решение 28); альтернатива не нужна",
    ),
    (
        "func f(h: Half) {\n\tmatch h {\n\t\tcase Full(_) {}\n\t\tcase Gone(_), Lost(..) {}\n\t}\n}\n",
        4,
        "к ветви не подходит ни одно значение: у типа Never значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(h: Half) {\n\tmatch h {\n\t\tcase Full(_) {}\n\t\tcase Full(1), Gone(_) {}\n\t}\n}\n",
        4,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(h: Half) {\n\tmatch h {\n\t\tcase Full(1) {}\n\t\tcase Full(2) {}\n\t\tcase Full(1), Full(2), Gone(_) {}\n\t\telse {}\n\t}\n}\n",
        5,
        "ветвь недостижима: все её значения разобраны ветвями выше (решение 7)",
    ),
    (
        "func f(v: Void) {\n\tmatch v {\n\t\tcase x {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Void значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(v: Void) {\n\tmatch v {\n\t\telse {}\n\t\telse {}\n\t}\n}\n",
        3,
        "к else не подходит ни одно значение: у типа Void значений нет (решение 28); else не нужен",
    ),
    (
        "func f(x: Never) {\n\tmatch x {\n\t\tcase y {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Never значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(t: (bool, Never)) {\n\tmatch t {\n\t\tcase u {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Never значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(t: (bool, Never)) {\n\tmatch t {\n\t\tcase (true, n) {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Never значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(a: Array[(i64, Void), 2]) {\n\tmatch a {\n\t\tcase b {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Void значений нет (решение 28); ветвь не нужна",
    ),
    (
        "func f(s: Nothing) {\n\tmatch s {\n\t\tcase t {}\n\t}\n}\n",
        3,
        "к ветви не подходит ни одно значение: у типа Nothing значений нет (решение 28); ветвь не нужна",
    ),
    // образцы вариантов (12.42, 13.11, решение 3)
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у варианта E.Num есть значение: case Num(…)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase E.Num {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у варианта E.Num есть значение: case E.Num(…)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Empty() {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у варианта E.Empty нет значения: case Empty",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(a, b) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "вариант E.Num несёт одно значение без имени: case Num(образец)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(..) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "вариант E.Num несёт одно значение без имени: case Num(образец)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(x: 1) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "вариант E.Num несёт одно значение без имени: case Num(образец)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(x, b) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "в образце варианта E.Named на этом месте поле a: имя без двоеточия совпадает с именем поля (13.11)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(b: _, a: _) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "в образце варианта E.Named на этом месте поле a, а не b: поля идут в порядке объявления (13.11)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(c: _, ..) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у варианта E.Named нет поля c",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(1, ..) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "в образце варианта E.Named поле a пишется именем a или a: образец (13.11)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(a, b, _) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у варианта E.Named полей: 2, а в образце больше",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(a,\n\t\t\tb,\n\t\t\tc: _) {}\n\t\telse {}\n\t}\n}\n",
        5,
        "у варианта E.Named полей: 2, а в образце больше",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(a) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "в образце варианта E.Named нет поля b: остальные поля пропускает .. (12.42)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(.., b) {}\n\t\telse {}\n\t}\n}\n",
        3,
        ".. стоит последним (12.42)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(..,\n\t\t\tb) {}\n\t\telse {}\n\t}\n}\n",
        4,
        ".. стоит последним (12.42)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Named(a, ..,\n\t\t\t..) {}\n\t\telse {}\n\t}\n}\n",
        3,
        ".. стоит последним (12.42)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase E.Num(_) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался Op, здесь E",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Op.Mul {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у перечисления Op нет варианта Mul",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Mul(x) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у перечисления Op нет варианта Mul",
    ),
    (
        "func f(p: P) {\n\tmatch p {\n\t\tcase P(x, y) {}\n\t}\n}\n",
        3,
        "не входит в шаг 2: образец записи (решение 13)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase Num(n) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "образец Num(…) — вариант, а значение типа i64",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase Some(n) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "не входит в шаг 2: Option",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase a.b.c {}\n\t\telse {}\n\t}\n}\n",
        3,
        "в образце вариант пишется V или E.V",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase a.b.c(x) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "в образце вариант пишется V(…) или E.V(…)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase Ok(n) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "не входит в шаг 2: Result",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase P.x {}\n\t\telse {}\n\t}\n}\n",
        3,
        "P — запись, а не перечисление",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase i64.x {}\n\t\telse {}\n\t}\n}\n",
        3,
        "i64 — не перечисление пакета",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Zz.x {}\n\t\telse {}\n\t}\n}\n",
        3,
        "нет перечисления Zz",
    ),
    // образцы-литералы (решение 10з, 12.56)
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 1.5 {}\n\t\telse {}\n\t}\n}\n",
        3,
        "дробное число 1.5 в образце: у дробных нет полного равенства (решение 10з)",
    ),
    (
        "func f(x: f64) {\n\tmatch x {\n\t\tcase 1 {}\n\t\telse {}\n\t}\n}\n",
        3,
        "образец-число на месте дробного f64: у дробных нет полного равенства (решение 10з)",
    ),
    (
        "func f(b: u8) {\n\tmatch b {\n\t\tcase 300 {}\n\t\telse {}\n\t}\n}\n",
        3,
        "число 300 не помещается в u8 (12.56)",
    ),
    (
        "func f(b: u8) {\n\tmatch b {\n\t\tcase -1 {}\n\t\telse {}\n\t}\n}\n",
        3,
        "у беззнакового u8 нет смены знака (9.18)",
    ),
    (
        "func f(c: bool) {\n\tmatch c {\n\t\tcase 1 {}\n\t\telse {}\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 'a' {}\n\t\telse {}\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался i64, здесь char",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase true {}\n\t\telse {}\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался i64, здесь bool",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase \"a\" {}\n\t\telse {}\n\t}\n}\n",
        3,
        "не входит в шаг 2: строки",
    ),
    (
        "func f(c: char) {\n\tmatch c {\n\t\tcase '\\u{D800}' {}\n\t\telse {}\n\t}\n}\n",
        3,
        "\\u{D800} — не скалярное значение Unicode",
    ),
    // кортежи (13.10)
    (
        "func f(t: (bool, bool)) {\n\tmatch t {\n\t\tcase (a, b, c) {}\n\t}\n}\n",
        3,
        "образец из 3 частей, а значение типа (bool, bool)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase (a, b) {}\n\t}\n}\n",
        3,
        "образец из 2 частей, а значение типа i64",
    ),
    // имена образцов (12.93, 12.43)
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase x {}\n\t}\n}\n",
        3,
        "имя x уже объявлено в строке 1: затенения нет (12.93)",
    ),
    (
        "func f(t: (i64, i64)) {\n\tmatch t {\n\t\tcase (a, a) {}\n\t}\n}\n",
        3,
        "имя a уже объявлено в строке 3: затенения нет (12.93)",
    ),
    (
        "func f(t: (bool, bool)) {\n\tmatch t {\n\t\tcase (a, true), (false, b) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "имя b связывает альтернатива 2, а первая — нет: альтернативы связывают одинаковые имена (12.43)",
    ),
    (
        "func f(t: (bool, bool)) {\n\tmatch t {\n\t\tcase (a, true), (false, _) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "альтернатива 2 не связывает имя a: альтернативы связывают одинаковые имена (12.43)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(n), Named(_, b: n) {}\n\t\telse {}\n\t}\n}\n",
        3,
        "имя n в альтернативе 2 типа f64, а в первой — i64: альтернативы связывают одинаковые типы (12.43)",
    ),
    (
        "func f(t: (i64, i64)) {\n\tmatch t {\n\t\tcase (a, b), (a, a) {}\n\t}\n}\n",
        3,
        "имя a в альтернативе 2 связано дважды (12.93)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase 0 {}\n\t\tcase n if n > 0 {}\n\t\telse {\n\t\t\tlet y = n\n\t\t}\n\t}\n}\n",
        6,
        "нет имени n",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase n {}\n\t}\n\tlet y = n\n}\n",
        5,
        "нет имени n",
    ),
    // условие ветви (12.43, решение 14)
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase n if 1 {}\n\t\telse {}\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase n if forever() {}\n\t\telse {}\n\t}\n}\n",
        3,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    // имена из образца не меняются, кроме имён из match mut (решение 16)
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase n {\n\t\t\tn = 1\n\t\t}\n\t}\n}\n",
        4,
        "n — имя из образца: оно не меняется, кроме имён из match mut (решение 16)",
    ),
    (
        "func f(take q: Q) {\n\tmatch take q {\n\t\tcase r {\n\t\t\tr = Q(z: 1)\n\t\t}\n\t}\n}\n",
        4,
        "r — имя из образца: оно не меняется, кроме имён из match mut (решение 16)",
    ),
    (
        "func f(e: E) {\n\tmatch e {\n\t\tcase Num(n) {\n\t\t\tbump(mut n)\n\t\t}\n\t\telse {}\n\t}\n}\n",
        4,
        "n — имя из образца: оно не меняется, кроме имён из match mut (решение 16)",
    ),
    // match mut — только у места, которое меняется (раздел 5)
    (
        "func f() {\n\tmatch mut two(1, 2) {\n\t\tcase n {}\n\t}\n}\n",
        2,
        "match mut — только у места: переменной, параметра, их поля или элемента",
    ),
    (
        "func f() {\n\tlet x = 1\n\tmatch mut x {\n\t\tcase n {}\n\t}\n}\n",
        3,
        "x объявлено через let: его значение не меняется (12.12)",
    ),
    (
        "func f(x: i64) {\n\tmatch mut x {\n\t\tcase n {}\n\t}\n}\n",
        2,
        "x — параметр для чтения: его не меняют (12.58)",
    ),
    (
        "func f(x: i64) {\n\tmatch x {\n\t\tcase n {\n\t\t\tmatch mut n {\n\t\t\t\tcase m {}\n\t\t\t}\n\t\t}\n\t}\n}\n",
        4,
        "n — имя из образца: оно не меняется, кроме имён из match mut (решение 16)",
    ),
    // значение match (решения 4, 7, 14, 24)
    (
        "func f() {\n\tmatch forever() {\n\t\tcase _ {}\n\t}\n}\n",
        2,
        "выражение типа Never значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
    (
        "func f(o: Op) {\n\tlet v: bool = match o {\n\t\tcase Add { 1 }\n\t\telse { 2 }\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(o: Op) {\n\tlet v = match o {\n\t\tcase Add { 1 }\n\t\telse { true }\n\t}\n}\n",
        3,
        "тип не совпадает: ожидался bool, здесь целое число",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {\n\t\t\t1\n\t\t}\n\t\telse {}\n\t}\n}\n",
        4,
        "значение типа i64 не используется: выбросить можно только результат вызова функции (решение 4)",
    ),
    (
        "func f(o: Op) -> i64 {\n\tmatch o {\n\t\tcase Add { 1 }\n\t\telse {\n\t\t\tlet y = 2\n\t\t}\n\t}\n}\n",
        5,
        "блок должен дать значение типа i64, а его последняя инструкция значения не даёт",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {\n\t\t\treturn\n\t\t}\n\t\telse {\n\t\t\treturn\n\t\t}\n\t}\n\tlet x = 1\n}\n",
        10,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    (
        "func f(v: Void) {\n\tmatch v {\n\t}\n\tlet x = 1\n}\n",
        4,
        "сюда выполнение не дойдёт: инструкция в строке 2 всегда уходит (решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {\n\t\t\tlet z = zz\n\t\t}\n\t\telse {}\n\t}\n}\n",
        4,
        "нет имени zz",
    ),
    // недостижимое и полнота проверяются раньше тел ветвей (раздел 7)
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {\n\t\t\tlet z = zz\n\t\t}\n\t\tcase Add {}\n\t\tcase Sub {}\n\t}\n}\n",
        6,
        "ветвь недостижима: все её значения разобраны ветвью в строке 3 (решение 7)",
    ),
    (
        "func f(o: Op) {\n\tmatch o {\n\t\tcase Add {\n\t\t\tlet z = zz\n\t\t}\n\t}\n}\n",
        2,
        "match разбирает не все значения: например, Op.Sub не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    ),
    // временное значение типа Never уходит раньше match: match по нему — недостижимый код
    (
        "func f() {\n\tmatch nothing() {\n\t}\n}\n",
        2,
        "выражение типа Void значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
    ),
];

#[test]
fn wrong_matches_are_rejected() {
    let prelude_lines = PRELUDE.lines().count() as u32;
    for (body, line, message) in MATCH_REJECTED {
        let text = format!("{}{}", PRELUDE, body);
        let message = shift_lines(message, prelude_lines);
        rejected(&text, prelude_lines + line, &message);
    }
}

// ───────── match: правильные программы ─────────

use nulixc::core::{LocalKind, Match, PatternKind};

const MATCH: &str = include_str!("check/match.nx");

/// `match` в конце тела функции: значение тела или, у тела типа `()`, последняя инструкция.
fn body_match<'p>(p: &'p Program, name: &str) -> &'p Match {
    let block = &func(p, name).body.block;
    let last = match block.value.as_deref() {
        Some(e) => Some(e),
        None => match block.stmts.last().map(|st| &st.kind) {
            Some(StmtKind::Expr(e)) => Some(e),
            _ => None,
        },
    };
    match last.map(|e| &e.kind) {
        Some(ExprKind::Match(m)) => m,
        _ => panic!("тело {} кончается не match", name),
    }
}

#[test]
fn match_program_is_accepted() {
    let p = accepted(MATCH);
    let (op, expr, wrap) = (type_id(&p, "Op"), type_id(&p, "Expr"), type_id(&p, "Wrap"));
    // имена из образцов — по ветвям, в порядке проверки
    assert_eq!(
        local_types(&p, "eval"),
        pairs(&[
            ("e", "Expr"),
            ("n", "i64"),
            ("n", "i64"),
            ("n", "i64"),
            ("op", "Op"),
            ("left", "i64"),
            ("right", "i64"),
        ])
    );
    // режим match — у каждого имени из образца (12.62, решение 16)
    let kind = |f: &str, k: usize| func(&p, f).body.locals[k].kind;
    assert_eq!(kind("eval", 1), LocalKind::Bound(Mode::Read));
    assert_eq!(kind("bump_num", 1), LocalKind::Bound(Mode::Mut));
    assert_eq!(kind("unwrap_num", 1), LocalKind::Bound(Mode::Take));
    assert_eq!(body_match(&p, "bump_num").mode, Mode::Mut);
    // альтернативы связывают одно имя одним номером (12.43); условие — у ветви
    let eval = body_match(&p, "eval");
    let alternatives: Vec<&PatternKind> = eval.cases[0]
        .patterns
        .iter()
        .map(|pat| match &pat.kind {
            PatternKind::Variant(_, _, items) => &items[0].kind,
            other => other,
        })
        .collect();
    assert_eq!(alternatives, [&PatternKind::Bind(1), &PatternKind::Bind(1)]);
    assert!(eval.cases[0].guard.is_some() && eval.cases[1].guard.is_none());
    // поля варианта — по объявлению, пропущенные — `_`
    let at = line_of(MATCH, "case Bin(op: Div");
    assert_eq!(
        eval.cases[3].patterns[0].kind,
        PatternKind::Variant(
            expr,
            2,
            vec![
                pattern(PatternKind::Variant(op, 3, Vec::new()), Type::Named(op), at),
                pattern(PatternKind::Skip, I64, at),
                pattern(PatternKind::Int(0), I64, at),
            ]
        )
    );
    // `..` пропускает остальные поля
    match &body_match(&p, "pair_kind").cases[2].patterns[0].kind {
        PatternKind::Variant(id, 1, items) => {
            assert_eq!(*id, wrap);
            let kinds: Vec<&PatternKind> = items.iter().map(|i| &i.kind).collect();
            assert_eq!(
                kinds,
                [&PatternKind::Variant(op, 0, Vec::new()), &PatternKind::Skip]
            );
        }
        other => panic!("ждали Wrap.Pair, а здесь {:?}", other),
    }
    // числа и символы — значениями
    let sign = body_match(&p, "sign");
    let ints: Vec<&PatternKind> = sign.cases[1].patterns.iter().map(|x| &x.kind).collect();
    assert_eq!(ints, [&PatternKind::Int(-1), &PatternKind::Int(-2)]);
    assert!(sign.otherwise.is_some());
    assert_eq!(
        body_match(&p, "digit").cases[2].patterns[0].kind,
        PatternKind::Char('A')
    );
    // тип match без своего типа задаёт место (решение 24)
    assert_eq!(
        local_types(&p, "weight"),
        pairs(&[("op", "Op"), ("w", "u16"), ("m", "u16")])
    );
    // match без ветвей по перечислению без вариантов уходит (9.36)
    let absurd = body_match(&p, "absurd");
    assert!(absurd.cases.is_empty() && absurd.otherwise.is_none());
    assert_eq!(func(&p, "absurd").body.block.ty, Type::Never);
    // match инструкцией: тип ветвей — ()
    let touch = &func(&p, "touch").body.block;
    assert!(touch.value.is_none());
    assert_eq!(p.tests.len(), 1);
}

fn pattern(kind: PatternKind, ty: Type, line: u32) -> nulixc::core::Pattern {
    nulixc::core::Pattern { kind, ty, line }
}

#[test]
fn guards_and_alternatives_are_accepted() {
    // альтернативы с условием друг друга не закрывают: при ложном условии проверка идёт к
    // следующей альтернативе (решение 27)
    accepted("func f(t: (i64, i64)) -> i64 {\n\tmatch t {\n\t\tcase (x, _), (_, x) if x > 0 { x }\n\t\telse { 0 }\n\t}\n}\n");
    // ветвь с условием не закрывает ту же ветвь без условия
    accepted("func f(x: i64) -> i64 {\n\tmatch x {\n\t\tcase n if n > 0 { n }\n\t\tcase n { -n }\n\t}\n}\n");
    // bool без else; одинаковые имена в альтернативах
    accepted("func f(t: (bool, i64)) -> i64 {\n\tmatch t {\n\t\tcase (true, n), (false, n) { n }\n\t}\n}\n");
    // имя из match mut меняется и отдаётся как mut; место — поле
    accepted(&format!(
        "{}func g(mut p: P) {{\n\tmatch mut p.x {{\n\t\tcase n {{\n\t\t\tbump(mut n)\n\t\t\tn += 1\n\t\t}}\n\t}}\n}}\n",
        PRELUDE
    ));
    // match в скобках, стоящий инструкцией, — тоже инструкция (решение 4)
    accepted(&format!(
        "{}func g(o: Op) {{\n\t(match o {{\n\t\tcase Add {{ two(1, 2) }}\n\t\telse {{ two(3, 4) }}\n\t}})\n}}\n",
        PRELUDE
    ));
}

#[test]
fn integer_matches_are_exact() {
    // у u8 256 значений: перечислены все — match полон без else
    let mut all = String::from("func f(x: u8) -> u8 {\n\tmatch x {\n");
    for v in 0..=255 {
        all.push_str(&format!("\t\tcase {} {{ {} }}\n", v, v));
    }
    all.push_str("\t}\n}\n");
    accepted(&all);
    // без одного — пример называет его
    let without = all.replace("\t\tcase 77 { 77 }\n", "");
    rejected(
        &without,
        2,
        "match разбирает не все значения: например, 77 не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    );
    // у символов пример — сначала буквы и цифры, затем по порядку кодов; невидимый знак —
    // `\u{…}`
    let mut chars = String::from("func f(c: char) {\n\tmatch c {\n");
    for c in ('a'..='z').chain('0'..='9').chain('A'..='Z') {
        chars.push_str(&format!("\t\tcase '{}' {{}}\n", c));
    }
    chars.push_str("\t}\n}\n");
    rejected(
        &chars,
        2,
        "match разбирает не все значения: например, '\\u{0}' не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    );
    // у i8 пример ищется и среди отрицательных
    let mut signed = String::from("func f(x: i8) {\n\tmatch x {\n");
    for v in 0..=127 {
        signed.push_str(&format!("\t\tcase {} {{}}\n", v));
    }
    signed.push_str("\t}\n}\n");
    rejected(
        &signed,
        2,
        "match разбирает не все значения: например, -1 не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    );
}

#[test]
fn wide_matches_run_on_a_small_stack() {
    // поиск полезности идёт без рекурсии: широкий кортеж и много ветвей проходят на стеке 1 МБ
    let n = 50_000;
    let ty = vec!["bool"; n].join(", ");
    let first = vec!["true"; n].join(", ");
    let text = format!(
        "func f(t: ({})) -> i64 {{\n\tmatch t {{\n\t\tcase ({}) {{ 1 }}\n\t\telse {{ 0 }}\n\t}}\n}}\n",
        ty, first
    );
    if let Err((line, message)) = run_on_small_stack(text) {
        panic!("отвергнуто, строка {}: {}", line, message);
    }
    // без else пример — кортеж, где первое значение false, а остальные любые
    let text = format!(
        "func f(t: ({})) {{\n\tmatch t {{\n\t\tcase ({}) {{}}\n\t}}\n}}\n",
        ty, first
    );
    match run_on_small_stack(text) {
        Err((2, message)) => assert!(
            message.starts_with("match разбирает не все значения: например, (false, _, _"),
            "{}",
            &message[..message.len().min(200)]
        ),
        Err((line, message)) => panic!("строка {}: {}", line, &message[..message.len().min(200)]),
        Ok(_) => panic!("принято"),
    }
    let mut many = String::from("func f(x: i64) -> i64 {\n\tmatch x {\n");
    for v in 0..2_000 {
        many.push_str(&format!("\t\tcase {} {{ {} }}\n", v, v));
    }
    many.push_str("\t\telse { -1 }\n\t}\n}\n");
    if let Err((line, message)) = run_on_small_stack(many) {
        panic!("отвергнуто, строка {}: {}", line, message);
    }
}

#[test]
fn values_that_cannot_exist_are_not_counted() {
    // у варианта с частью типа без значений значений нет: его не пишут (решение 28)
    let decls = "enum Void {}\nenum M {\n\tA\n\tB(Never)\n\tC(x: i64, v: Void)\n}\nstruct S {\n\tn: Never\n}\nstruct Z {\n\ta: Array[Never, 0]\n}\nstruct S2 {\n\ta: i64\n\tn: Never\n}\nenum Q2 {\n\tV(x: i64, v: Void)\n}\n";
    accepted(&format!(
        "{}func f(m: M) -> i64 {{\n\tmatch m {{\n\t\tcase A {{ 1 }}\n\t}}\n}}\n",
        decls
    ));
    // по значению типа без значений match пишется без ветвей, и его тип — Never; значения нет,
    // если его нет хотя бы у одной части записи, варианта или кортежа
    let p = accepted(&format!(
        "{}func f(s: S) -> i64 {{\n\tmatch s {{\n\t}}\n}}\nfunc g(s: S2) -> i64 {{\n\tmatch s {{\n\t}}\n}}\nfunc h(q: Q2) -> i64 {{\n\tmatch q {{\n\t}}\n}}\nfunc k(t: (bool, Q2)) -> i64 {{\n\tmatch t {{\n\t}}\n}}\nfunc a(t: Array[Void, 3]) -> i64 {{\n\tmatch t {{\n\t}}\n}}\n",
        decls
    ));
    for name in ["f", "g", "h", "k", "a"] {
        assert_eq!(func(&p, name).body.block.ty, Type::Never, "{}", name);
    }
    // у пустого массива значение есть всегда — сам пустой массив
    let inhabited: Vec<bool> = p.types.iter().map(|t| t.inhabited).collect();
    assert_eq!(inhabited, [false, true, false, true, false, false]);
    rejected(
        &format!(
            "{}func f(z: Z) {{\n\tmatch z {{\n\t}}\n}}\n",
            decls
        ),
        decls.lines().count() as u32 + 2,
        "match без ветвей бывает только по значению типа без значений, а у Z значения есть (решение 28); добавьте ветвь или else",
    );
    // пример неразобранного значения — из тех, что бывают
    let first_empty = "enum K {\n\tX(Never)\n\tY\n\tZ\n}\n";
    rejected(
        &format!(
            "{}func f(k: K) {{\n\tmatch k {{\n\t\tcase Z {{}}\n\t}}\n}}\n",
            first_empty
        ),
        7,
        "match разбирает не все значения: например, K.Y не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    );
    rejected(
        &format!(
            "{}func f(t: (K, bool)) {{\n\tmatch t {{\n\t\tcase (Z, _) {{}}\n\t\tcase (_, true) {{}}\n\t}}\n}}\n",
            first_empty
        ),
        7,
        "match разбирает не все значения: например, (K.Y, false) не подходит ни к одной ветви (9.35); добавьте ветвь или else",
    );
}

#[test]
fn places_of_type_never_are_matched_without_branches() {
    // место типа Never — параметр, его поле, элемент — сравнивают в match без ветвей при любом
    // режиме: ветвей у такого match нет, и недостижимого кода тоже (решение 28)
    let p = accepted(&format!(
        "{}func f(x: Never) -> i64 {{\n\tmatch x {{\n\t}}\n}}\nfunc g(take x: Never) -> i64 {{\n\tmatch take x {{\n\t}}\n}}\nfunc h(mut x: Never) -> i64 {{\n\tmatch mut x {{\n\t}}\n}}\nfunc k(s: Nothing) -> i64 {{\n\tmatch s.n {{\n\t}}\n}}\nfunc a(t: Array[Never, 2]) -> i64 {{\n\tmatch t[1] {{\n\t}}\n}}\n",
        PRELUDE
    ));
    for name in ["f", "g", "h", "k", "a"] {
        assert_eq!(func(&p, name).body.block.ty, Type::Never, "{}", name);
    }
    // после такого match выполнение не идёт (решение 7)
    rejected(
        &format!(
            "{}func f(x: Never) {{\n\tmatch x {{\n\t}}\n\tlet y = 1\n}}\n",
            PRELUDE
        ),
        PRELUDE.lines().count() as u32 + 4,
        &format!(
            "сюда выполнение не дойдёт: инструкция в строке {} всегда уходит (решение 7)",
            PRELUDE.lines().count() + 2
        ),
    );
}

#[test]
fn match_records_the_mode_it_compares_with() {
    // место без слова — чтение, временное значение образец забирает сам (12.62)
    let p = accepted(&format!(
        "{}func f(x: i64) -> i64 {{\n\tmatch x {{\n\t\tcase n {{ n }}\n\t}}\n}}\nfunc g() -> i64 {{\n\tmatch two(1, 2) {{\n\t\tcase n {{ n }}\n\t}}\n}}\nfunc h(q: Q) -> i64 {{\n\tmatch (q.z) {{\n\t\tcase n {{ n }}\n\t}}\n}}\n",
        PRELUDE
    ));
    assert_eq!(body_match(&p, "f").mode, Mode::Read);
    assert_eq!(
        func(&p, "f").body.locals[1].kind,
        LocalKind::Bound(Mode::Read)
    );
    assert_eq!(body_match(&p, "g").mode, Mode::Take);
    assert_eq!(
        func(&p, "g").body.locals[0].kind,
        LocalKind::Bound(Mode::Take)
    );
    assert_eq!(body_match(&p, "h").mode, Mode::Read);
}

#[test]
fn examples_are_written_as_literals() {
    // символы перед примером: сначала буквы и цифры, затем по порядку кодов до нужного
    let order: Vec<char> = ('a'..='z')
        .chain('0'..='9')
        .chain('A'..='Z')
        .chain((0..=0x7F).filter_map(char::from_u32))
        .collect();
    for (target, text) in [
        ('\'', "'\\''"),
        ('\\', "'\\\\'"),
        (' ', "' '"),
        ('\u{7F}', "'\\u{7F}'"),
    ] {
        let mut src = String::from("func f(c: char) {\n\tmatch c {\n");
        let mut seen = std::collections::HashSet::new();
        for &c in &order {
            if c == target {
                break;
            }
            if !seen.insert(c) {
                continue;
            }
            let lit = match c {
                c if c.is_ascii_graphic() && c != '\'' && c != '\\' => format!("'{}'", c),
                c => format!("'\\u{{{:X}}}'", c as u32),
            };
            src.push_str(&format!("\t\tcase {} {{}}\n", lit));
        }
        src.push_str("\t}\n}\n");
        rejected(
            &src,
            2,
            &format!(
                "match разбирает не все значения: например, {} не подходит ни к одной ветви (9.35); добавьте ветвь или else",
                text
            ),
        );
    }
}

#[test]
fn wide_matches_are_checked_in_near_linear_time() {
    // ветви раскладываются по конструкторам за один проход: прежняя проверка шла
    // по ветвям выше для каждой ветви, и на таких match время росло квадратично
    let n = 20_000;
    let mut enum_all = String::from("enum Big {\n");
    for k in 0..n {
        enum_all.push_str(&format!("\tV{}\n", k));
    }
    enum_all.push_str("}\nfunc f(b: Big) -> i64 {\n\tmatch b {\n");
    for k in 0..n {
        enum_all.push_str(&format!("\t\tcase V{} {{ {} }}\n", k, k));
    }
    enum_all.push_str("\t}\n}\n");
    let mut pairs = String::from("func f(x: (i64, bool)) -> i64 {\n\tmatch x {\n");
    for k in 0..n {
        pairs.push_str(&format!("\t\tcase ({}, _) {{ {} }}\n", k, k));
    }
    pairs.push_str("\t\telse { -1 }\n\t}\n}\n");
    let mut full = String::from("func f(x: u16) -> i64 {\n\tmatch x {\n");
    for k in 0..=u16::MAX {
        full.push_str(&format!("\t\tcase {} {{ 1 }}\n", k));
    }
    full.push_str("\t}\n}\n");
    // с одной строкой без условия, которая разбирает всё, остальное не перебирается: 64 ветви
    // по кортежу из 32 bool, в каждой задано до трёх мест
    let mut tuple = String::from("func f(t: (");
    tuple.push_str(&vec!["bool"; 32].join(", "));
    tuple.push_str(")) -> i64 {\n\tmatch t {\n");
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..64 {
        let mut row = vec!["_"; 32];
        for _ in 0..3 {
            let c = (next() % 32) as usize;
            row[c] = if next() % 2 == 0 { "true" } else { "false" };
        }
        tuple.push_str(&format!("\t\tcase ({}) {{ 1 }}\n", row.join(", ")));
    }
    tuple.push_str("\t\telse { 0 }\n\t}\n}\n");
    for text in [enum_all, pairs, full, tuple] {
        let start = std::time::Instant::now();
        accepted(&text);
        // с большим запасом: на этих match квадратичная проверка шла минутами
        assert!(start.elapsed().as_secs() < 30, "{:?}", start.elapsed());
    }
}

#[test]
fn match_branches_without_values_give_no_type() {
    // ветвь, которая уходит, тип не задаёт: литерал в другой ветви — i64 (решение 24)
    let p = accepted(&format!(
        "{}func g(o: Op) -> i64 {{\n\tlet v = match o {{\n\t\tcase Add {{ forever() }}\n\t\telse {{ 1 }}\n\t}}\n\tv\n}}\n",
        PRELUDE
    ));
    assert_eq!(local_types(&p, "g"), pairs(&[("o", "Op"), ("v", "i64")]));
}
