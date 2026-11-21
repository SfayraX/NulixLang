//! Сверка типизации проверщика IR с интерпретатором. Из комплекта внешней оценки проверщика (20–21 ноября 2026).
//!
//! Три семейства сгенерированных модулей. Для каждого модуля проверщик и интерпретатор дают
//! вердикт, и вердикты сравниваются:
//!   * «дыра»   — проверщик принял, а интерпретатор при исполнении теста находит неверный IR;
//!   * «строже» — проверщик отверг, а интерпретатор неверного IR не находит.
//!
//! Ожидается ноль и того и другого. Правила типизации в спецификации (`spec/ir-core.md`):
//! арифметика — целые и дробные; `neg` — знаковые и дробные; `and`/`or`/`xor`/`not` — целые и
//! `bool`; сдвиги — целые, величина сдвига — целое любого типа; `eq`/`ne` — все скаляры;
//! `lt`/`le`/`gt`/`ge` — целые, дробные и символы; результат сравнения — `bool`.
//!
//! Семейства:
//!   1. операции: 12 скалярных типов × 18 операций × тип результата (тип операции или `bool`),
//!      у сдвигов ещё четыре типа величины сдвига — всего 552 модуля;
//!   2. преобразования `conv.A.B` для всех пар скалярных типов — 144 модуля;
//!   3. константы: 40 записей × 12 скалярных типов и `()` — 520 модулей, из них 481 читается
//!      (читатель отвергает суррогаты Unicode, `nan` и `inf`: 3 записи × 13 типов).
//!
//! Запуск (из каталога `compiler/`):
//!     cargo test --test verifier_typing -- --nocapture

use nulixc::interp::{self, Limits, Stop};
use nulixc::ir_text::parse;
use nulixc::verify::verify;

const TYPES: [&str; 12] = [
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool", "char",
];

const BIN: [&str; 16] = [
    "add", "sub", "mul", "div", "rem", "and", "or", "xor", "shl", "shr", "eq", "ne", "lt", "le",
    "gt", "ge",
];
const UN: [&str; 2] = ["neg", "not"];

/// Записи констант; для типа `()` и остальных одни и те же.
const LITS: [&str; 40] = [
    "0",
    "1",
    "-1",
    "127",
    "128",
    "-128",
    "-129",
    "255",
    "256",
    "32767",
    "32768",
    "65535",
    "65536",
    "2147483647",
    "2147483648",
    "-2147483648",
    "4294967295",
    "4294967296",
    "9223372036854775807",
    "9223372036854775808",
    "-9223372036854775808",
    "-9223372036854775809",
    "18446744073709551615",
    "18446744073709551616",
    "1.5",
    "-0.0",
    "0.0",
    "1e39",
    "1e308",
    "3.4028235e38",
    "3.4028236e38",
    "true",
    "false",
    "'a'",
    "'\\u{10FFFF}'",
    "'\\u{D800}'",
    "()",
    "1.0",
    "nan",
    "inf",
];

/// Константа, подходящая типу.
fn lit(t: &str) -> &'static str {
    match t {
        "f32" | "f64" => "1.5",
        "bool" => "true",
        "char" => "'a'",
        _ => "1",
    }
}

fn module(body: String) -> String {
    format!("ir 1\n\n{}", body)
}

/// Тело модуля с одной функцией `f` и тестом, который её вызывает.
fn with_test(f_body: String) -> String {
    module(format!(
        "func f() {{\n{}}}\ntest \"t\" {{ call f() }}\n",
        f_body
    ))
}

fn gen_ops() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for t in TYPES {
        let results: Vec<&str> = if t == "bool" {
            vec!["bool"]
        } else {
            vec![t, "bool"]
        };
        for op in BIN.iter().chain(UN.iter()) {
            for r in &results {
                if *op == "shl" || *op == "shr" {
                    for c in ["u8", "i64", "f64", "bool"] {
                        let body = format!("  c: {c} = const {}\n  a: {t} = const {}\n  x: {r} = {op}.{t}(copy a, copy c)\n", lit(c), lit(t));
                        out.push((format!("{op}_{t}_to_{r}_cnt_{c}"), with_test(body)));
                    }
                } else if UN.contains(op) {
                    let body =
                        format!("  a: {t} = const {}\n  x: {r} = {op}.{t}(copy a)\n", lit(t));
                    out.push((format!("{op}_{t}_to_{r}"), with_test(body)));
                } else {
                    let body = format!("  a: {t} = const {}\n  b: {t} = const {}\n  x: {r} = {op}.{t}(copy a, copy b)\n", lit(t), lit(t));
                    out.push((format!("{op}_{t}_to_{r}"), with_test(body)));
                }
            }
        }
    }
    out
}

fn gen_conv() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for a in TYPES {
        for b in TYPES {
            let body = format!(
                "  a: {a} = const {}\n  x: {b} = conv.{a}.{b}(copy a)\n",
                lit(a)
            );
            out.push((format!("conv_{a}_{b}"), with_test(body)));
        }
    }
    out
}

fn gen_consts() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for t in TYPES.iter().copied().chain(std::iter::once("unit")) {
        let ty = if t == "unit" { "()" } else { t };
        for l in LITS {
            let body = format!("  x: {ty} = const {l}\n");
            out.push((format!("const_{t}_{l}"), with_test(body)));
        }
    }
    out
}

#[derive(Default)]
struct Tally {
    total: usize,
    unreadable: usize,
    holes: Vec<String>,
    stricter: Vec<String>,
}

fn judge(items: Vec<(String, String)>) -> Tally {
    let mut t = Tally::default();
    for (name, text) in items {
        t.total += 1;
        let m = match parse(&text) {
            Ok(m) => m,
            Err(_) => {
                t.unreadable += 1;
                continue;
            }
        };
        let accepted = verify(&m).is_ok();
        let invalid = match interp::run_tests(&m, Limits::default()) {
            Ok(rs) => rs
                .iter()
                .any(|r| matches!(r.stop, Some(Stop::InvalidIr(_)))),
            Err(Stop::InvalidIr(_)) => true,
            Err(_) => false,
        };
        if accepted && invalid {
            t.holes.push(name);
        } else if !accepted && !invalid {
            t.stricter.push(name);
        }
    }
    t
}

fn report(title: &str, t: &Tally) {
    println!(
        "{:<12} модулей {:>4} | не читается {:>3} | сверено {:>4} | дыр {} | проверщик строже интерпретатора {}",
        title,
        t.total,
        t.unreadable,
        t.total - t.unreadable,
        t.holes.len(),
        t.stricter.len()
    );
    for n in &t.holes {
        println!("  ДЫРА: {}", n);
    }
    for n in &t.stricter {
        println!("  СТРОЖЕ: {}", n);
    }
}

#[test]
fn typing_agrees_with_interpreter() {
    let ops = judge(gen_ops());
    let conv = judge(gen_conv());
    let consts = judge(gen_consts());
    report("операции", &ops);
    report("conv", &conv);
    report("константы", &consts);
    // размеры семейств — чтобы тест не вырождался молча
    assert_eq!((ops.total, ops.unreadable), (552, 0), "семейство операций");
    assert_eq!((conv.total, conv.unreadable), (144, 0), "семейство conv");
    assert_eq!(
        (consts.total, consts.unreadable),
        (520, 39),
        "семейство констант"
    );
    for t in [&ops, &conv, &consts] {
        assert!(
            t.holes.is_empty(),
            "проверщик принял то, что интерпретатор находит неверным: {:?}",
            t.holes
        );
        assert!(
            t.stricter.is_empty(),
            "проверщик отверг то, что интерпретатор исполняет: {:?}",
            t.stricter
        );
    }
}
