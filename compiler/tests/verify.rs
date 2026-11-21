//! Проверщик IR.
//!
//! Правильный IR — программы `tests/ir/` и пример из `spec/ir-core.md` — принимается, и всё
//! принятое интерпретатор исполняет без «неверного IR». Неверный IR отвергается: и тот, где
//! ошибку находит интерпретатор при исполнении (`invalid.ir`), и тот, где её видно только при
//! проверке (`rejected.ir`), и неверные объявления. Наконец, правильный IR портится по одной
//! отметке или инструкции так, что он гарантированно становится неверным, и проверщик
//! отвергает каждую такую порчу.

use nulixc::interp::{self, Limits, Stop, Value};
use nulixc::ir::*;
use nulixc::ir_text::{self, parse};
use nulixc::verify::verify;
use std::collections::HashMap;

const FLOW: &str = include_str!("ir/flow.ir");
const DATA: &str = include_str!("ir/data.ir");
const ORDER: &str = include_str!("ir/order.ir");
const RUNNER: &str = include_str!("ir/runner.ir");
const INVALID: &str = include_str!("ir/invalid.ir");
const REJECTED: &str = include_str!("ir/rejected.ir");

// ───────── помощники ─────────

fn module(text: &str) -> Module {
    match parse(text) {
        Ok(m) => m,
        Err(e) => panic!("строка {}: {}\n{}", e.line, e.message, text),
    }
}

/// Первый пример IR из `spec/ir-core.md` — функция gcd.
fn gcd_example() -> Module {
    let path = format!("{}/../spec/ir-core.md", env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(&path).expect("не найден spec/ir-core.md");
    let start = doc.find("```ir\n").expect("в документе нет примера IR") + "```ir\n".len();
    let end = start + doc[start..].find("```").expect("пример не закрыт");
    module(&doc[start..end])
}

/// Правильный IR.
fn corpus() -> Vec<(&'static str, Module)> {
    vec![
        ("flow.ir", module(FLOW)),
        ("data.ir", module(DATA)),
        ("order.ir", module(ORDER)),
        ("runner.ir", module(RUNNER)),
        ("gcd из spec/ir-core.md", gcd_example()),
    ]
}

/// Модуль из всех объявлений текста, кроме функций-случаев; из случаев остаётся `keep`.
fn only(m: &Module, cases: &[&str], keep: Option<&str>) -> Module {
    let decls = m
        .decls
        .iter()
        .filter(|d| match d {
            Decl::Func(f) => !cases.contains(&f.name.as_str()) || keep == Some(f.name.as_str()),
            _ => true,
        })
        .cloned()
        .collect();
    Module {
        version: m.version,
        decls,
    }
}

/// Каждый случай отвергается в своей функции и с ожидаемым сообщением; без случаев модуль
/// принимается — значит, помощники правильны и случай отвергнут за свою ошибку.
#[track_caller]
fn each_case_rejected(text: &str, cases: &[(&str, &str)]) {
    let full = module(text);
    let names: Vec<&str> = cases.iter().map(|c| c.0).collect();
    assert_eq!(verify(&only(&full, &names, None)), Ok(()), "помощники");
    for (case, fragment) in cases {
        let m = only(&full, &names, Some(case));
        assert!(
            m.decls.len() > only(&full, &names, None).decls.len(),
            "нет функции {}",
            case
        );
        match verify(&m) {
            Err(e)
                if e.at.starts_with(&format!("функция {}", case))
                    && e.message.contains(fragment) => {}
            other => panic!(
                "{}: ожидался отказ «{}», а вышло {:?}",
                case, fragment, other
            ),
        }
    }
}

// ───────── правильный IR ─────────

#[test]
fn the_correct_corpus_is_accepted() {
    for (name, m) in corpus() {
        assert_eq!(verify(&m), Ok(()), "{}", name);
    }
}

/// Пробные значения скалярного типа; `None` — тип не скалярный.
fn samples(t: &Type) -> Option<Vec<Value>> {
    Some(match t {
        Type::Scalar(s) => match s.int_range() {
            Some((lo, hi)) => {
                let mut ns = vec![0, 1, 7, -3, lo, hi];
                ns.retain(|n| lo <= *n && *n <= hi);
                ns.dedup();
                ns.into_iter().map(|n| Value::Int(*s, n)).collect()
            }
            None => match s {
                Scalar::F32 => vec![Value::F32(0.0), Value::F32(1.5), Value::F32(-2.0)],
                Scalar::F64 => vec![Value::F64(0.0), Value::F64(1.5), Value::F64(-2.0)],
                Scalar::Bool => vec![Value::Bool(false), Value::Bool(true)],
                _ => vec![Value::Char('a'), Value::Char('я')],
            },
        },
        Type::Unit => vec![Value::Unit],
        _ => return None,
    })
}

/// Все наборы аргументов: по одному значению из каждого множества.
fn product(sets: &[Vec<Value>]) -> Vec<Vec<Value>> {
    let mut out = vec![Vec::new()];
    for set in sets {
        let mut next = Vec::new();
        for prefix in &out {
            for v in set {
                let mut args: Vec<Value> = prefix.clone();
                args.push(v.clone());
                next.push(args);
            }
        }
        out = next;
    }
    out
}

/// Принятое проверщиком интерпретатор исполняет без «неверного IR»: тесты модулей
/// запускаются, функции со скалярными параметрами вызываются на пробных значениях. Аварии и
/// исчерпание бюджета — законные исходы.
#[test]
fn the_interpreter_runs_what_the_verifier_accepts() {
    let limits = Limits {
        steps: 20_000,
        depth: 100,
    };
    let mut calls = 0;
    for (name, m) in corpus() {
        assert_eq!(verify(&m), Ok(()), "{}", name);
        for r in interp::run_tests(&m, limits).expect("модуль исполняется") {
            assert!(
                !matches!(r.stop, Some(Stop::InvalidIr(_))),
                "{}: тест «{}»: {:?}",
                name,
                r.name,
                r.stop
            );
        }
        for d in &m.decls {
            let f = match d {
                Decl::Func(f) if f.params.iter().all(|p| p.mode != Mode::Mut) => f,
                _ => continue,
            };
            let sets: Option<Vec<Vec<Value>>> = f.params.iter().map(|p| samples(&p.ty)).collect();
            for args in product(&sets.unwrap_or_default()) {
                if args.len() != f.params.len() {
                    continue;
                }
                calls += 1;
                let r = interp::call(&m, &f.name, args.clone(), limits);
                assert!(
                    !matches!(r, Err(Stop::InvalidIr(_))),
                    "{}: {}({:?}): {:?}",
                    name,
                    f.name,
                    args,
                    r
                );
            }
        }
    }
    assert!(calls > 100, "вызовов всего {}", calls);
}

// ───────── неверный IR ─────────

/// `invalid.ir`: ошибку находит и интерпретатор при исполнении (тест `interp.rs`), но
/// проверщик отвергает IR раньше — целиком, не исполняя.
const INVALID_CASES: &[(&str, &str)] = &[
    ("read_empty", "copy x: значения нет"),
    ("copy_owned", "copy t: тип Token не простой"),
    // найдено прогоном ослаблений из внешней оценки проверщика 21 ноября 2026
    ("copy_tuple_owned", "copy t: тип (i64, Token) не простой"),
    ("overwrite", "в t ещё есть непростое значение: сначала drop"),
    ("leak", "значение в t не отдано и не уничтожено"),
    ("half_leak", "значение в p.1 не отдано и не уничтожено"),
    ("no_return", "конец тела достижим, а результат i64"),
    ("bare_return", "return без значения, а результат i64"),
    ("no_case", "switch x неполон"),
    ("bad_label", "нет цикла Nowhere"),
    ("big_const", "константа 300 не помещается в u8"),
    ("f32_inexact", "не точное значение f32"),
    ("borrow_value", "бывает только аргументом вызова"),
    ("unknown", "нет функции nobody"),
    ("arity", "функция takes_mut берёт аргументов: 1; дано: 0"),
    ("wrong_mode", "аргумент не подходит к режиму параметра x"),
    ("drop_param", "drop t: параметр t функции не принадлежит"),
    ("move_param", "move t: параметр t функции не принадлежит"),
    ("write_param", "запись в x: x — параметр для чтения"),
    ("wrong_variant", "s.Rect.w: вариант в s здесь неизвестен"),
    ("discard", "результат вызова unit_result не используется"),
    ("redeclare", "x уже объявлена с типом i64, а здесь bool"),
    ("lie", "return без значения, а результат Never"),
    ("left_to_right_bad", "x пересекается с x"),
    ("read_mut", "borrow mut x: x — параметр для чтения"),
    ("stale_after_call", "d.Left.0: вариант в d здесь неизвестен"),
    ("stale_in_loop", "d.Left.0: вариант в d здесь неизвестен"),
    (
        "stale_index",
        "a[i].Left.0: вариант в a[i] здесь неизвестен",
    ),
    (
        "stale_element",
        "a[i].Left.0: вариант в a[i] здесь неизвестен",
    ),
    ("stale_target", "d.Left.0: вариант в d здесь неизвестен"),
    ("lent_index", "номер элемента в a[i] берётся из i"),
    (
        "stale_after_branch",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_after_inner_switch",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_in_outer_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_after_define",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    ("index_empty", "в номере элемента i нет значения"),
    ("borrow_after_lend", "p.a пересекается с p"),
    ("borrow_empty", "borrow t: значения нет"),
    ("lend_empty", "borrow mut x: значения нет"),
    ("move_while_lent", "p пересекается с p.a"),
    ("drop_twice", "drop t: значения нет"),
    ("if_empty", "в условии c нет значения"),
    ("return_leak", "значение в t не отдано и не уничтожено"),
    ("write_empty_array", "запись в xs[0]: массива нет"),
    (
        "write_into_empty",
        "запись в p.a: нет значения, частью которого оно было бы",
    ),
    (
        "move_mut_part",
        "move p.a: параметр p функции не принадлежит",
    ),
    ("switch_empty", "в switch d нет значения"),
    ("if_not_bool", "значение типа i64 там, где ждут bool"),
    ("return_wrong_type", "константа не подходит к типу i64"),
    ("call_result_type", "значение типа i64 там, где ждут bool"),
    ("make_wrong_type", "значение типа Token там, где ждут Pair"),
    ("make_missing_field", "make Pair: указаны не все поля"),
    ("array_length", "массив не подходит к типу Array[i64, 3]"),
    ("conv_operand_type", "значение типа i64 там, где ждут i32"),
    (
        "stale_lent_in_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    ("move_twice", "move t: значения нет"),
    (
        "redeclare_owned",
        "в t ещё есть непростое значение: сначала drop",
    ),
    (
        "stale_lent_in_define",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_redefined_in_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_lent_in_assign",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    ("stale_if_in_loop", "d.Left.0: вариант в d здесь неизвестен"),
    (
        "stale_case_in_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_else_in_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_after_whole_write",
        "a[0].Left.0: вариант в a[0] здесь неизвестен",
    ),
    ("local_empties", "copy x: значения нет"),
    ("local_unknown_type", "нет типа Q"),
    ("conv_result_type", "значение типа i64 там, где ждут i32"),
    (
        "borrow_wrong_type",
        "значение типа Other там, где ждут Token",
    ),
    ("lend_wrong_type", "значение типа Other там, где ждут Token"),
    (
        "make_duplicate_field",
        "make Pair: поля указаны не так, как объявлены",
    ),
    (
        "make_named_payload",
        "make Opt.Yes: значения не подходят к объявлению варианта",
    ),
    ("shift_count_bool", "величина сдвига — целое"),
    ("swap_empty", "swap t, u: в t нет значения целиком"),
    ("swap_partial", "swap p, q: в p нет значения целиком"),
    ("swap_read_param", "swap t, u: t — параметр для чтения"),
    ("swap_types", "swap t, o: типы Token и Other разные"),
    ("stale_after_swap", "d.Left.0: вариант в d здесь неизвестен"),
    (
        "stale_swap_in_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_after_swap_second",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
    (
        "stale_swap_second_in_loop",
        "d.Left.0: вариант в d здесь неизвестен",
    ),
];

#[test]
fn ir_the_interpreter_finds_invalid_is_rejected() {
    each_case_rejected(INVALID, INVALID_CASES);
}

/// `rejected.ir`: ошибку видно при проверке; интерпретатор многое из этого исполняет.
const REJECTED_CASES: &[(&str, &str)] = &[
    ("dead_after_return", "инструкция недостижима"),
    ("dead_after_never", "инструкция недостижима"),
    ("dead_after_loop", "инструкция недостижима"),
    ("never_as_value", "вызывают только инструкцией call"),
    (
        "if_without_after",
        "ветки if доходят до конца, а состояние после не записано",
    ),
    (
        "if_with_extra_after",
        "после if записано состояние, а ни одна ветка до конца не доходит",
    ),
    (
        "loop_without_after",
        "у цикла L не записано состояние после",
    ),
    (
        "loop_with_extra_after",
        "после цикла L записано состояние, а выхода из него нет",
    ),
    (
        "after_names_moved",
        "в конце ветки then: записанное состояние называет t, а значения там нет",
    ),
    (
        "after_forgets_owned",
        "в конце ветки else: в t непростое значение",
    ),
    (
        "after_names_partial",
        "записанное состояние называет p целиком, а часть значения перемещена",
    ),
    ("after_names_param", "параметры чтения и mut не называются"),
    ("after_twice", "места i и i повторяются или пересекаются"),
    (
        "after_overlaps",
        "места p и p.a повторяются или пересекаются",
    ),
    ("after_all_parts", "названы все части p"),
    (
        "after_names_payload",
        "o.Yes.0: называются переменные, поля записей и элементы кортежей",
    ),
    (
        "after_names_element",
        "xs[0]: называются переменные, поля записей и элементы кортежей",
    ),
    ("after_names_unknown", "нет переменной nobody"),
    (
        "loop_entry_mismatch",
        "на входе в цикл L: записанное состояние называет t",
    ),
    (
        "continue_mismatch",
        "при переходе к следующему проходу цикла L: в t непростое значение",
    ),
    (
        "loop_end_mismatch",
        "в конце тела цикла L — перед следующим проходом: в t непростое значение",
    ),
    (
        "break_mismatch",
        "на выходе из цикла L: записанное состояние называет t",
    ),
    (
        "payload_left_in_branch",
        "в конце ветки case Both: в x.Both.b непростое значение",
    ),
    ("else_unreachable", "else в switch o недостижим"),
    ("bool_else_unreachable", "else в switch b недостижим"),
    ("case_twice", "case 1 повторён"),
    ("switch_on_float", "switch по x типа f64 нельзя"),
    ("wrong_label", "метка case не подходит к switch n"),
    (
        "label_out_of_range",
        "case 300: значение не помещается в u8",
    ),
    ("same_label_nested", "цикл L вложен в цикл с той же меткой"),
    ("read_part_then_lend_whole", "p пересекается с p.a"),
    ("lend_same_twice", "p.a пересекается с p.a"),
    ("lend_unknown_elements", "xs[i] пересекается с xs[0]"),
    ("lend_same_element", "xs[1] пересекается с xs[1]"),
    (
        "move_element",
        "move xs[0]: элемент массива отдельно не забирают",
    ),
    (
        "drop_element",
        "drop xs[0]: элемент массива отдельно не уничтожают",
    ),
    (
        "replace_element",
        "непростое значение в элементе массива не заменить",
    ),
    (
        "replace_mut_part",
        "в p.a ещё есть непростое значение: сначала drop",
    ),
    ("index_not_i64", "номер элемента k — не переменная i64"),
    (
        "payload_after_rewrite",
        "o.Yes.0.id: вариант в o здесь неизвестен",
    ),
    (
        "payload_after_switch",
        "o.Yes.0.id: вариант в o здесь неизвестен",
    ),
    ("payload_in_other_case", "o.Yes.0.id: в o вариант No"),
    ("op_missing", "у типа u8 нет операции neg"),
    ("bool_ordered", "у типа bool нет операции lt"),
    ("char_arithmetic", "у типа char нет операции add"),
    ("operand_type", "значение типа f64 там, где ждут i64"),
    ("result_type", "значение типа i64 там, где ждут bool"),
    ("shift_by_float", "величина сдвига типа f64 — не целое"),
    ("conv_bool", "преобразования conv.bool.i64 нет"),
    ("conv_char_float", "преобразования conv.char.f64 нет"),
    ("const_kind", "константа не подходит к типу bool"),
    (
        "redeclare_param",
        "x — параметр, объявить его заново нельзя",
    ),
    ("tuple_shape", "кортеж не подходит к типу (i64, bool)"),
    (
        "swap_unknown_variant",
        "o.Yes.0: вариант в o здесь неизвестен",
    ),
];

#[test]
fn ir_wrong_only_for_the_verifier_is_rejected() {
    each_case_rejected(REJECTED, REJECTED_CASES);
}

#[test]
fn declarations_are_checked() {
    let cases = [
        (
            "struct A { a: A }",
            "тип A содержит сам себя без косвенности",
        ),
        (
            "struct A { b: B }\n\nstruct B { a: A }",
            "содержит сам себя без косвенности",
        ),
        (
            "enum List { Nil, Cons(head: i64, tail: List) }",
            "тип List содержит сам себя",
        ),
        ("struct G { cells: Array[G, 2] }", "тип G содержит сам себя"),
        ("struct P { q: (i64, P) }", "тип P содержит сам себя"),
        (
            "struct T { id: i64 }\n\ncopy struct C { t: T }",
            "простая запись с непростым полем t",
        ),
        (
            "struct T { id: i64 }\n\ncopy enum E { A(T) }",
            "простое перечисление с непростым значением в варианте A",
        ),
        ("struct P { x: i64, x: i64 }", "поле x повторено"),
        ("enum E { A, A }", "вариант A повторён"),
        ("enum E { A(x: i64, x: i64) }", "поле x повторено"),
        ("struct P { q: Q }", "поле q: нет типа Q"),
        ("enum E { A(Q) }", "вариант A: нет типа Q"),
        ("func f(a: i64, a: i64) { }", "параметр a повторён"),
        ("func f(a: Q) { }", "параметр a: нет типа Q"),
        (
            "func f() -> Q {\n  crash \"нет\"\n}",
            "результат: нет типа Q",
        ),
        ("func f() uses (fs) { }", "требований uses на шаге 2 нет"),
        ("struct A { }\n\nenum A { }", "тип A объявлен дважды"),
        // повтор имени записи; найдено внешней оценкой проверщика 20 ноября 2026
        ("struct A { }\n\nstruct A { }", "тип A объявлен дважды"),
        ("func f() { }\n\nfunc f() { }", "функция f объявлена дважды"),
    ];
    for (text, fragment) in cases {
        let m = module(&format!("ir 1\n\n{}\n", text));
        match verify(&m) {
            Err(e) if e.message.contains(fragment) => {}
            other => panic!(
                "{}\nожидался отказ «{}», а вышло {:?}",
                text, fragment, other
            ),
        }
    }
    // без косвенности, но и без круга: ромб и вложенные кортежи и массивы
    let ok = "ir 1\n\nstruct D { x: i64 }\n\nstruct B { d: D }\n\nstruct C { d: (D, Array[D, 2]) }\n\nstruct A { b: B, c: C }\n";
    assert_eq!(verify(&module(ok)), Ok(()));
    // чего текстом не записать: другая версия, кортеж из одного элемента, пустой список полей
    let mut m = module("ir 1\n\nenum E { A(x: i64) }\n\nfunc f(t: (i64, i64)) { }\n");
    let mut other = m.clone();
    other.version = 2;
    assert!(
        matches!(verify(&other), Err(e) if e.message.contains("версия IR 2 не поддерживается"))
    );
    let mut other = m.clone();
    if let Decl::Func(f) = &mut other.decls[1] {
        f.params[0].ty = Type::Tuple(vec![Type::Scalar(Scalar::I64)]);
    }
    assert!(
        matches!(verify(&other), Err(e) if e.message.contains("в кортеже меньше двух элементов"))
    );
    if let Decl::Enum(e) = &mut m.decls[0] {
        e.variants[0].payload = Payload::Fields(Vec::new());
    }
    assert!(matches!(verify(&m), Err(e) if e.message.contains("у варианта A пустой список полей")));
}

/// Чего текстом не записать: читатель IR такого не пропустит, но модуль можно собрать и
/// программно, например переводом. Найдено независимым ревью 20 ноября 2026.
#[test]
fn what_text_cannot_express_is_checked() {
    let mut m = module("ir 1\n\nfunc f() -> i64 {\n  x: i64 = const 1\n  r: i64 = neg.i64(copy x)\n  return copy r\n}\n");
    if let Decl::Func(f) = &mut m.decls[0] {
        if let Stmt::Define {
            value: Rhs::Op { args, .. },
            ..
        } = &mut f.body.stmts[1]
        {
            args.push(args[0].clone());
        }
    }
    assert!(
        matches!(verify(&m), Err(e) if e.message.contains("операция neg берёт операндов: 1; дано: 2"))
    );
    let mut m =
        module("ir 1\n\nstruct P { a: i64 }\n\nfunc f() {\n  p: P = make P(a: const 1)\n}\n");
    if let Decl::Func(f) = &mut m.decls[1] {
        if let Stmt::Define {
            value: Rhs::Make { args, .. },
            ..
        } = &mut f.body.stmts[0]
        {
            *args = None;
        }
    }
    assert!(matches!(verify(&m), Err(e) if e.message.contains("make P: поля записи — в скобках")));
    // Дробная константа — только конечная: бесконечность и NaN читатель не пропустит, но модуль
    // можно собрать программно. Найдено внешней оценкой проверщика 20 ноября 2026.
    for (ty, bits) in [
        ("f64", f64::NAN.to_bits()),
        ("f64", f64::INFINITY.to_bits()),
        ("f64", f64::NEG_INFINITY.to_bits()),
        ("f32", f64::INFINITY.to_bits()),
    ] {
        let mut m = module(&format!(
            "ir 1\n\nfunc f() -> {ty} {{\n  x: {ty} = const 1.5\n  return copy x\n}}\n"
        ));
        if let Decl::Func(f) = &mut m.decls[0] {
            if let Stmt::Define {
                value: Rhs::Use(Operand::Const(Const::Float(b))),
                ..
            } = &mut f.body.stmts[0]
            {
                *b = bits;
            }
        }
        assert!(
            matches!(verify(&m), Err(e) if e.message.contains("дробная константа должна быть конечной")),
            "{} {:x}",
            ty,
            bits
        );
    }
}

/// Каждое имя записывается текстом IR. Проверяются имена объявлений — остальные имена ведут к
/// объявленным. Найдено независимым ревью 20 ноября 2026: программный модуль с пустым именем
/// проходил проверку, а его текст не читался обратно.
#[test]
fn names_must_have_a_text_form() {
    const BASE: &str = "ir 1\n\nstruct P { x: i64 }\n\nenum E { A, B(y: i64) }\n\nfunc f(p: i64) -> i64 {\n  local t: i64\n  v: i64 = const 1\n  loop L [held: v] {\n    break L\n  } [held: v]\n  return copy v\n}\n";
    fn func(m: &mut Module) -> &mut Func {
        match &mut m.decls[2] {
            Decl::Func(f) => f,
            _ => panic!("третье объявление — функция"),
        }
    }
    type Spoil = fn(&mut Module);
    let cases: [(&str, Spoil); 11] = [
        ("запись \"\"", |m| {
            if let Decl::Struct(s) = &mut m.decls[0] {
                s.name = String::new();
            }
        }),
        ("поле \"a`b\"", |m| {
            if let Decl::Struct(s) = &mut m.decls[0] {
                s.fields[0].name = "a`b".to_string();
            }
        }),
        ("перечисление \"E\\u{7}\"", |m| {
            if let Decl::Enum(e) = &mut m.decls[1] {
                e.name = "E\u{7}".to_string();
            }
        }),
        ("вариант \"\"", |m| {
            if let Decl::Enum(e) = &mut m.decls[1] {
                e.variants[0].name = String::new();
            }
        }),
        ("поле \"x\\n\"", |m| {
            if let Decl::Enum(e) = &mut m.decls[1] {
                if let Payload::Fields(fs) = &mut e.variants[1].payload {
                    fs[0].name = "x\n".to_string();
                }
            }
        }),
        ("функция \"a..b\"", |m| {
            func(m).name = "a..b".to_string()
        }),
        ("функция \"a`b\"", |m| {
            func(m).name = "a`b".to_string()
        }),
        ("параметр \"\"", |m| {
            func(m).params[0].name = String::new()
        }),
        ("переменная \"t\\t\"", |m| {
            if let Stmt::Local { name, .. } = &mut func(m).body.stmts[0] {
                *name = "t\t".to_string();
            }
        }),
        ("переменная \"\"", |m| {
            if let Stmt::Define { name, .. } = &mut func(m).body.stmts[1] {
                *name = String::new();
            }
        }),
        ("метка цикла \"\"", |m| {
            if let Stmt::Loop { label, .. } = &mut func(m).body.stmts[2] {
                *label = String::new();
            }
        }),
    ];
    let base = module(BASE);
    assert_eq!(verify(&base), Ok(()));
    for (what, spoil) in cases {
        let mut m = base.clone();
        spoil(&mut m);
        assert_ne!(m, base, "{}: порча не применилась", what);
        let expected = format!("{}: имя нельзя записать текстом IR", what);
        match verify(&m) {
            Err(e) if e.message.contains(&expected) => {}
            other => panic!("ожидался отказ «{}», а вышло {:?}", expected, other),
        }
        // и правда: текст такого модуля обратно не читается или читается другим модулем
        assert_ne!(parse(&ir_text::print(&m)).ok(), Some(m), "{}", what);
    }
    // необычные, но записываемые имена принимаются, и текст читается обратно тем же модулем
    let mut m = base.clone();
    if let Decl::Struct(s) = &mut m.decls[0] {
        s.name = "a.b".to_string();
    }
    func(&mut m).name = "Point.norm".to_string();
    if let Stmt::Local { name, .. } = &mut func(&mut m).body.stmts[0] {
        *name = "два слова".to_string();
    }
    func(&mut m).params[0].name = "copy".to_string();
    assert_eq!(verify(&m), Ok(()));
    assert_eq!(parse(&ir_text::print(&m)), Ok(m));
}

/// Проверка рекурсивна по вложенности, поэтому идёт на своём потоке с большим стеком: вызов с
/// маленького стека не падает там, где маленького стека не хватило бы. Найдено независимым
/// ревью 20 ноября 2026: раньше проверщик работал на стеке вызывающего. Предел вложенности IR
/// вводится на шаге 5.
#[test]
fn deep_nesting_does_not_overflow_the_callers_stack() {
    let n = 3000;
    let text = format!(
        "ir 1\n\nfunc f(c: bool) {{\n{}{}}}\n",
        "if c { ".repeat(n),
        "} else { } [held:] ".repeat(n)
    );
    let result = nulixc::stack::run(move || {
        let m = module(&text);
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(1 << 20)
                .spawn_scoped(s, || verify(&m))
                .expect("поток")
                .join()
                .expect("проверщик упал")
        })
    });
    assert_eq!(result, Ok(()));
}

// ───────── порча ─────────

/// Где инструкция: номер объявления, путь по вложенным блокам — номер инструкции и номер её
/// блока — и номер инструкции в последнем блоке.
#[derive(Clone, Debug)]
struct Addr {
    decl: usize,
    path: Vec<(usize, usize)>,
    stmt: usize,
}

fn sub_blocks(s: &Stmt) -> Vec<&Block> {
    match s {
        Stmt::If {
            then, otherwise, ..
        } => vec![then, otherwise],
        Stmt::Switch {
            cases, otherwise, ..
        } => cases
            .iter()
            .map(|c| &c.body)
            .chain(otherwise.iter())
            .collect(),
        Stmt::Loop { body, .. } => vec![body],
        _ => Vec::new(),
    }
}

fn sub_block_mut(s: &mut Stmt, k: usize) -> &mut Block {
    match s {
        Stmt::If {
            then, otherwise, ..
        } => {
            if k == 0 {
                then
            } else {
                otherwise
            }
        }
        Stmt::Switch {
            cases, otherwise, ..
        } => {
            if k < cases.len() {
                &mut cases[k].body
            } else {
                otherwise.as_mut().expect("нет else")
            }
        }
        Stmt::Loop { body, .. } => body,
        _ => panic!("у инструкции нет блоков"),
    }
}

fn decl_body(d: &Decl) -> Option<&Block> {
    match d {
        Decl::Func(f) => Some(&f.body),
        Decl::Test(t) => Some(&t.body),
        _ => None,
    }
}

fn decl_body_mut(d: &mut Decl) -> &mut Block {
    match d {
        Decl::Func(f) => &mut f.body,
        Decl::Test(t) => &mut t.body,
        _ => panic!("у объявления нет тела"),
    }
}

/// Все инструкции модуля.
fn addrs(m: &Module) -> Vec<Addr> {
    fn walk(b: &Block, decl: usize, path: &mut Vec<(usize, usize)>, out: &mut Vec<Addr>) {
        for (i, s) in b.stmts.iter().enumerate() {
            out.push(Addr {
                decl,
                path: path.clone(),
                stmt: i,
            });
            for (k, sb) in sub_blocks(s).into_iter().enumerate() {
                path.push((i, k));
                walk(sb, decl, path, out);
                path.pop();
            }
        }
    }
    let mut out = Vec::new();
    for (i, d) in m.decls.iter().enumerate() {
        if let Some(b) = decl_body(d) {
            walk(b, i, &mut Vec::new(), &mut out);
        }
    }
    out
}

fn block_at<'a>(m: &'a mut Module, a: &Addr) -> &'a mut Block {
    let mut b = decl_body_mut(&mut m.decls[a.decl]);
    for &(i, k) in &a.path {
        b = sub_block_mut(&mut b.stmts[i], k);
    }
    b
}

fn stmt_at<'a>(m: &'a mut Module, a: &Addr) -> &'a mut Stmt {
    &mut block_at(m, a).stmts[a.stmt]
}

fn stmt<'a>(m: &'a Module, a: &Addr) -> &'a Stmt {
    let mut b = decl_body(&m.decls[a.decl]).expect("у объявления нет тела");
    for &(i, k) in &a.path {
        b = sub_blocks(&b.stmts[i])[k];
    }
    &b.stmts[a.stmt]
}

/// Что известно о функции, где стоит инструкция.
struct Scope {
    vars: HashMap<String, Type>,
    modes: HashMap<String, Mode>,
    result: Type,
}

fn scope(d: &Decl) -> Scope {
    fn locals(b: &Block, vars: &mut HashMap<String, Type>) {
        for s in &b.stmts {
            if let Stmt::Local { name, ty } | Stmt::Define { name, ty, .. } = s {
                vars.insert(name.clone(), ty.clone());
            }
            for sb in sub_blocks(s) {
                locals(sb, vars);
            }
        }
    }
    let mut sc = Scope {
        vars: HashMap::new(),
        modes: HashMap::new(),
        result: Type::Unit,
    };
    if let Decl::Func(f) = d {
        for p in &f.params {
            sc.vars.insert(p.name.clone(), p.ty.clone());
            sc.modes.insert(p.name.clone(), p.mode);
        }
        sc.result = f.result.clone();
    }
    if let Some(b) = decl_body(d) {
        locals(b, &mut sc.vars);
    }
    sc
}

fn struct_decl<'m>(m: &'m Module, name: &str) -> Option<&'m StructDecl> {
    m.decls.iter().find_map(|d| match d {
        Decl::Struct(s) if s.name == name => Some(s),
        _ => None,
    })
}

fn enum_decl<'m>(m: &'m Module, name: &str) -> Option<&'m EnumDecl> {
    m.decls.iter().find_map(|d| match d {
        Decl::Enum(e) if e.name == name => Some(e),
        _ => None,
    })
}

fn is_copy(m: &Module, t: &Type) -> bool {
    match t {
        Type::Scalar(_) | Type::Unit => true,
        Type::Tuple(ts) => ts.iter().all(|t| is_copy(m, t)),
        Type::Array(t, _) => is_copy(m, t),
        Type::Named(n) => struct_decl(m, n)
            .map(|s| s.copy)
            .or_else(|| enum_decl(m, n).map(|e| e.copy))
            .unwrap_or(false),
    }
}

fn place_type(m: &Module, sc: &Scope, p: &Place) -> Option<Type> {
    let mut ty = sc.vars.get(&p.base)?.clone();
    let mut i = 0;
    while i < p.path.len() {
        ty = match (&p.path[i], &ty) {
            (Seg::Field(f), Type::Named(n)) => match struct_decl(m, n) {
                Some(s) => s.fields.iter().find(|x| x.name == *f)?.ty.clone(),
                None => {
                    let v = enum_decl(m, n)?.variants.iter().find(|v| v.name == *f)?;
                    i += 1;
                    match (&v.payload, p.path.get(i)?) {
                        (Payload::One(t), Seg::Pos(0)) => t.clone(),
                        (Payload::Fields(fs), Seg::Field(g)) => {
                            fs.iter().find(|x| x.name == *g)?.ty.clone()
                        }
                        _ => return None,
                    }
                }
            },
            (Seg::Pos(k), Type::Tuple(ts)) => ts.get(*k as usize)?.clone(),
            (Seg::Elem(_), Type::Array(t, _)) => t.as_ref().clone(),
            _ => return None,
        };
        i += 1;
    }
    Some(ty)
}

fn owned_type(m: &Module, sc: &Scope, p: &Place) -> bool {
    place_type(m, sc, p).is_some_and(|t| !is_copy(m, &t))
}

fn rhs_operands_mut<'a>(r: &'a mut Rhs, out: &mut Vec<&'a mut Operand>) {
    match r {
        Rhs::Use(o) | Rhs::Conv { arg: o, .. } => out.push(o),
        Rhs::Op { args, .. } | Rhs::Call { args, .. } | Rhs::Tuple(args) | Rhs::Array(args) => {
            out.extend(args.iter_mut())
        }
        Rhs::Make { args, .. } => out.extend(args.iter_mut().flatten().map(|a| &mut a.value)),
    }
}

/// Операнды самой инструкции, без вложенных блоков.
fn operands_mut(s: &mut Stmt) -> Vec<&mut Operand> {
    let mut out = Vec::new();
    match s {
        Stmt::Define { value, .. } | Stmt::Assign { value, .. } => {
            rhs_operands_mut(value, &mut out)
        }
        Stmt::Call { args, .. } => out.extend(args.iter_mut()),
        Stmt::Return(Some(o)) => out.push(o),
        _ => {}
    }
    out
}

/// Аргументы вызова в инструкции.
fn call_args_mut(s: &mut Stmt) -> Option<&mut Vec<Operand>> {
    match s {
        Stmt::Call { args, .. } => Some(args),
        Stmt::Define {
            value: Rhs::Call { args, .. },
            ..
        }
        | Stmt::Assign {
            value: Rhs::Call { args, .. },
            ..
        } => Some(args),
        _ => None,
    }
}

/// Записанные состояния инструкции: после слияния и в заголовке цикла.
fn records_mut(s: &mut Stmt) -> Vec<&mut State> {
    match s {
        Stmt::If { after, .. } | Stmt::Switch { after, .. } => after.iter_mut().collect(),
        Stmt::Loop { head, after, .. } => std::iter::once(head).chain(after.iter_mut()).collect(),
        _ => Vec::new(),
    }
}

/// Порча: для инструкции выдаёт все испорченные копии модуля.
type Spoil = fn(&Module, &Addr, &Scope) -> Vec<Module>;

/// Каждый операнд инструкции по очереди, заменённый, если `f` его заменяет.
fn each_operand(m: &Module, a: &Addr, f: &dyn Fn(&Operand) -> Option<Operand>) -> Vec<Module> {
    let n = operands_mut(&mut stmt(m, a).clone()).len();
    let mut out = Vec::new();
    for j in 0..n {
        let mut s = m.clone();
        let mut ops = operands_mut(stmt_at(&mut s, a));
        if let Some(new) = f(ops[j]) {
            *ops.swap_remove(j) = new;
            out.push(s);
        }
    }
    out
}

/// Каждое записанное состояние инструкции по очереди, изменённое `f`, если оно его меняет.
fn each_record(m: &Module, a: &Addr, f: &dyn Fn(&State) -> Vec<State>) -> Vec<Module> {
    let n = records_mut(&mut stmt(m, a).clone()).len();
    let mut out = Vec::new();
    for j in 0..n {
        let current = records_mut(&mut stmt(m, a).clone())[j].clone();
        for new in f(&current) {
            let mut s = m.clone();
            *records_mut(stmt_at(&mut s, a)).swap_remove(j) = new;
            out.push(s);
        }
    }
    out
}

/// S1: убран `drop` непростого значения.
fn drop_removed(m: &Module, a: &Addr, sc: &Scope) -> Vec<Module> {
    match stmt(m, a) {
        Stmt::Drop(p) if owned_type(m, sc, p) => {
            let mut s = m.clone();
            block_at(&mut s, a).stmts.remove(a.stmt);
            vec![s]
        }
        _ => Vec::new(),
    }
}

/// S2: `move` непростого значения стал `copy`.
fn move_to_copy(m: &Module, a: &Addr, sc: &Scope) -> Vec<Module> {
    each_operand(m, a, &|o| match o {
        Operand::Move(p) if owned_type(m, sc, p) => Some(Operand::Copy(p.clone())),
        _ => None,
    })
}

/// S3: `copy` из параметра чтения или `mut` стал `move`.
fn copy_to_move_from_param(m: &Module, a: &Addr, sc: &Scope) -> Vec<Module> {
    each_operand(m, a, &|o| match o {
        Operand::Copy(p) if matches!(sc.modes.get(&p.base), Some(Mode::Read | Mode::Mut)) => {
            Some(Operand::Move(p.clone()))
        }
        _ => None,
    })
}

/// S4: из записанного состояния убрано непростое место.
fn record_loses_owned(m: &Module, a: &Addr, sc: &Scope) -> Vec<Module> {
    each_record(m, a, &|r| {
        (0..r.held.len())
            .filter(|&k| owned_type(m, sc, &r.held[k]))
            .map(|k| {
                let mut r = r.clone();
                r.held.remove(k);
                r
            })
            .collect()
    })
}

/// S5: в записанное состояние добавлен повтор места или параметр чтения или `mut`.
fn record_gains(m: &Module, a: &Addr, sc: &Scope) -> Vec<Module> {
    let mut params: Vec<&String> = sc
        .modes
        .iter()
        .filter(|(_, mode)| **mode != Mode::Take)
        .map(|(name, _)| name)
        .collect();
    params.sort();
    each_record(m, a, &|r| {
        let mut out = Vec::new();
        if let Some(first) = r.held.first() {
            let mut r = r.clone();
            r.held.push(first.clone());
            out.push(r);
        }
        if let Some(name) = params.first() {
            let mut r = r.clone();
            r.held.push(Place {
                base: name.to_string(),
                path: Vec::new(),
            });
            out.push(r);
        }
        out
    })
}

/// S6: состояние после конструкции убрано, а где его не было — добавлено.
fn after_toggled(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let mut s = m.clone();
    match stmt_at(&mut s, a) {
        Stmt::If { after, .. } | Stmt::Switch { after, .. } | Stmt::Loop { after, .. } => {
            *after = match after {
                Some(_) => None,
                None => Some(State::default()),
            };
            vec![s]
        }
        _ => Vec::new(),
    }
}

/// S7: инструкция добавлена после безусловного выхода.
fn dead_inserted(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let never = |name: &str| {
        m.decls.iter().any(|d| match d {
            Decl::Func(f) if f.name == name => match &f.result {
                Type::Named(n) => enum_decl(m, n).is_some_and(|e| e.variants.is_empty()),
                _ => false,
            },
            _ => false,
        })
    };
    let exits = match stmt(m, a) {
        Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Crash(_) => true,
        Stmt::Call { func, .. } => never(func),
        Stmt::If { after, .. } | Stmt::Switch { after, .. } | Stmt::Loop { after, .. } => {
            after.is_none()
        }
        _ => false,
    };
    if !exits {
        return Vec::new();
    }
    let mut s = m.clone();
    block_at(&mut s, a)
        .stmts
        .insert(a.stmt + 1, Stmt::Crash("лишнее".to_string()));
    vec![s]
}

/// S8: у определения другой тип: `()`, а у определения `()` — `bool`.
fn define_retyped(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let mut s = m.clone();
    match stmt_at(&mut s, a) {
        Stmt::Define { ty, .. } => {
            *ty = match ty {
                Type::Unit => Type::Scalar(Scalar::Bool),
                _ => Type::Unit,
            };
            vec![s]
        }
        _ => Vec::new(),
    }
}

/// S9: у операции другой тип: целое — `f64`, дробное — `i64`, `bool` и символ — `i64`.
fn op_retyped(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let mut s = m.clone();
    match stmt_at(&mut s, a) {
        Stmt::Define {
            value: Rhs::Op { ty, .. },
            ..
        }
        | Stmt::Assign {
            value: Rhs::Op { ty, .. },
            ..
        } => {
            *ty = if ty.int_range().is_some() {
                Scalar::F64
            } else {
                Scalar::I64
            };
            vec![s]
        }
        _ => Vec::new(),
    }
}

/// S10: `break` или `continue` ведёт к циклу, которого нет.
fn label_lost(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let mut s = m.clone();
    match stmt_at(&mut s, a) {
        Stmt::Break(l) | Stmt::Continue(l) => {
            *l = "Нигде".to_string();
            vec![s]
        }
        _ => Vec::new(),
    }
}

/// S11: у `switch` убран `else`, а где его не было — добавлен.
fn else_toggled(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let mut s = m.clone();
    match stmt_at(&mut s, a) {
        Stmt::Switch { otherwise, .. } => {
            *otherwise = match otherwise {
                Some(_) => None,
                None => Some(Block::default()),
            };
            vec![s]
        }
        _ => Vec::new(),
    }
}

/// S12: ветка `case` повторена; S17: у `switch` без `else` убрана ветка.
fn cases_spoiled(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let mut out = Vec::new();
    if let Stmt::Switch {
        cases, otherwise, ..
    } = stmt(m, a)
    {
        if !cases.is_empty() {
            let mut s = m.clone();
            if let Stmt::Switch { cases, .. } = stmt_at(&mut s, a) {
                let first = cases[0].clone();
                cases.push(first);
            }
            out.push(s);
        }
        if otherwise.is_none() {
            for k in 0..cases.len() {
                let mut s = m.clone();
                if let Stmt::Switch { cases, .. } = stmt_at(&mut s, a) {
                    cases.remove(k);
                }
                out.push(s);
            }
        }
    }
    out
}

/// S13: в аргументе вызова `borrow` стал `borrow mut` и наоборот.
fn borrow_flipped(m: &Module, a: &Addr, _: &Scope) -> Vec<Module> {
    let n = call_args_mut(&mut stmt(m, a).clone()).map_or(0, |args| args.len());
    let mut out = Vec::new();
    for j in 0..n {
        let mut s = m.clone();
        let args = call_args_mut(stmt_at(&mut s, a)).expect("вызов");
        args[j] = match &args[j] {
            Operand::Borrow(p) => Operand::BorrowMut(p.clone()),
            Operand::BorrowMut(p) => Operand::Borrow(p.clone()),
            _ => continue,
        };
        out.push(s);
    }
    out
}

/// S16: у функции с результатом не `()` убран последний `return`.
fn final_return_removed(m: &Module, a: &Addr, sc: &Scope) -> Vec<Module> {
    let last = decl_body(&m.decls[a.decl]).map_or(0, |b| b.stmts.len());
    match stmt(m, a) {
        Stmt::Return(_) if a.path.is_empty() && a.stmt + 1 == last && sc.result != Type::Unit => {
            let mut s = m.clone();
            block_at(&mut s, a).stmts.remove(a.stmt);
            vec![s]
        }
        _ => Vec::new(),
    }
}

const SPOILS: [(&str, Spoil); 14] = [
    ("убран drop непростого значения", drop_removed),
    ("move непростого значения стал copy", move_to_copy),
    ("copy из параметра стал move", copy_to_move_from_param),
    ("из состояния убрано непростое место", record_loses_owned),
    ("в состояние добавлен повтор или параметр", record_gains),
    ("состояние после убрано или добавлено", after_toggled),
    ("инструкция после выхода", dead_inserted),
    ("у определения другой тип", define_retyped),
    ("у операции другой тип", op_retyped),
    ("метка цикла, которого нет", label_lost),
    ("else убран или добавлен", else_toggled),
    ("ветка case повторена или убрана", cases_spoiled),
    ("borrow и borrow mut переставлены", borrow_flipped),
    ("убран последний return", final_return_removed),
];

fn decl_name(d: &Decl) -> String {
    match d {
        Decl::Func(f) => format!("функция {}", f.name),
        Decl::Test(t) => format!("тест «{}»", t.name),
        Decl::Struct(s) => s.name.clone(),
        Decl::Enum(e) => e.name.clone(),
    }
}

#[test]
fn every_single_spoil_is_rejected() {
    let mut counts = vec![0usize; SPOILS.len()];
    for (name, m) in corpus() {
        assert_eq!(verify(&m), Ok(()), "{}", name);
        for a in addrs(&m) {
            let sc = scope(&m.decls[a.decl]);
            for (k, (what, spoil)) in SPOILS.iter().enumerate() {
                for s in spoil(&m, &a, &sc) {
                    counts[k] += 1;
                    assert!(
                        verify(&s).is_err(),
                        "{}: порча «{}» принята: {}, {:?}\n{}",
                        name,
                        what,
                        decl_name(&m.decls[a.decl]),
                        a,
                        ir_text::print(&s)
                    );
                }
            }
        }
    }
    for (k, (what, _)) in SPOILS.iter().enumerate() {
        assert!(counts[k] > 0, "порча «{}» ни разу не применена", what);
    }
}
