//! Предел вложенности и длинные цепочки: разборщик отвечает ошибкой или деревом, но не падает.
//!
//! Сверка с проверочным разборщиком ровно у предела — в `parser_golden.rs`. Здесь то, что
//! проверочному не под силу: тексты намного глубже предела и очень длинные цепочки.

use nulixc::parser::{parse_rule, MAX_DEPTH};
use nulixc::stack;

/// Способы вложить одно в другое: правило грамматики и текст с данным числом уровней.
fn kinds() -> Vec<(&'static str, &'static str, fn(usize) -> String)> {
    vec![
        ("круглые скобки", "expr", |n| {
            "(".repeat(n) + "1" + &")".repeat(n)
        }),
        ("списки", "expr", |n| "[".repeat(n) + &"]".repeat(n)),
        ("вызовы", "expr", |n| "f(".repeat(n) + &")".repeat(n)),
        ("блоки if", "expr", |n| {
            "if c { ".repeat(n) + &"}".repeat(n)
        }),
        ("блоки замыканий", "expr", |n| {
            "func() { ".repeat(n) + &"}".repeat(n)
        }),
        ("not", "expr", |n| "not ".repeat(n) + "x"),
        ("минус", "expr", |n| "- ".repeat(n) + "x"),
        ("try", "expr", |n| "try ".repeat(n) + "x"),
        ("if в условии if", "expr", |n| {
            "if ".repeat(n) + "c" + &" { }".repeat(n)
        }),
        ("match в заголовке match", "expr", |n| {
            "match ".repeat(n) + "x" + &" { }".repeat(n)
        }),
        ("provide в значении provide", "expr", |n| {
            "provide a = ".repeat(n) + "x" + &" { }".repeat(n)
        }),
        ("замыкание в limit", "expr", |n| {
            "func() limit a <= ".repeat(n) + "1" + &" { }".repeat(n)
        }),
        ("ref", "type", |n| "ref ".repeat(n) + "T"),
        ("mut ptr", "type", |n| "mut ptr ".repeat(n) + "T"),
        ("типы результата", "type", |n| {
            "func() -> ".repeat(n) + "T"
        }),
        ("аргументы типов", "type", |n| {
            "A[".repeat(n) + "T" + &"]".repeat(n)
        }),
        ("образцы", "pattern", |n| {
            "A(".repeat(n) + "_" + &")".repeat(n)
        }),
        ("целый файл", "file", |n| {
            String::from("x = ") + &"(".repeat(n) + "1" + &")".repeat(n) + "\n"
        }),
    ]
}

#[test]
fn limit_is_exact() {
    stack::run(|| {
        let limit = MAX_DEPTH as usize;
        for (name, rule, make) in kinds() {
            assert!(
                parse_rule(rule, &make(limit)).is_ok(),
                "{}: {} уровней должны разбираться",
                name,
                limit
            );
            let error = parse_rule(rule, &make(limit + 1)).unwrap_err();
            assert!(
                error.message.contains("вложенность глубже"),
                "{}: на {} уровнях ожидалась ошибка вложенности, получено: {}",
                name,
                limit + 1,
                error.message
            );
        }
    });
}

#[test]
fn far_beyond_the_limit_is_an_error_not_a_crash() {
    stack::run(|| {
        for (name, rule, make) in kinds() {
            let error = parse_rule(rule, &make(100_000)).unwrap_err();
            assert!(
                error.message.contains("вложенность глубже"),
                "{}: ожидалась ошибка вложенности, получено: {}",
                name,
                error.message
            );
        }
    });
}

/// Цепочки — не вложенность: длина им не помеха ни при разборе, ни при записи дерева,
/// ни при его уничтожении.
#[test]
fn long_chains_are_not_nesting() {
    stack::run(|| {
        let n = 200_000;
        let cases: Vec<(&str, &str, String)> = vec![
            ("сумма", "expr", String::from("a") + &" + a".repeat(n)),
            (
                "произведение и сумма",
                "expr",
                String::from("a") + &" * a - a".repeat(n),
            ),
            ("and", "expr", String::from("a") + &" and a".repeat(n)),
            (
                "побитовое или",
                "expr",
                String::from("a") + &" | a".repeat(n),
            ),
            (
                "вызовы подряд",
                "expr",
                String::from("a") + &".f()".repeat(n),
            ),
            (
                "индексы подряд",
                "expr",
                String::from("a") + &"[0]".repeat(n),
            ),
            (
                "имя через точки",
                "expr",
                String::from("a") + &".b".repeat(n),
            ),
            (
                "else if",
                "expr",
                String::from("if c { }") + &" else if c { }".repeat(n),
            ),
            (
                "элементы списка",
                "expr",
                String::from("[1") + &", 1".repeat(n) + "]",
            ),
            (
                "аргументы вызова",
                "expr",
                String::from("f(1") + &", 1".repeat(n) + ")",
            ),
            ("инструкции", "stmts", String::from("x") + &"; x".repeat(n)),
            (
                "ветви match",
                "expr",
                String::from("match x {") + &" case 1 { }".repeat(n) + " }",
            ),
            (
                "кортеж типов",
                "type",
                String::from("(T") + &", T".repeat(n) + ")",
            ),
            ("объявления", "file", "func f() { }\n".repeat(n)),
        ];
        for (name, rule, text) in cases {
            match parse_rule(rule, &text) {
                Ok(tree) => assert!(tree.len() > n, "{}: запись дерева слишком короткая", name),
                Err(error) => panic!("{}: строка {}: {}", name, error.line, error.message),
            }
        }
    });
}
