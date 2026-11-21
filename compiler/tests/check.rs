//! Проверка явного ядра в исходнике (`spec/core-check.md`): файл, объявления и сигнатуры.
//!
//! Правильные объявления принимаются, и проверенное дерево несёт их типы, режимы и простоту.
//! Каждое правило объявлений нарушается по одному разу в отдельной программе, и проверка
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
        "impl Copy for B {}\nfunc f(a: A) -> B {\n}\nstruct A {\n\tb: B\n}\nimpl Copy for A {}\nstruct B {\n\tx: i64\n}\n",
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
