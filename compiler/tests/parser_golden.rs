//! Сверка разборщика с проверочным разборщиком (Lark по `grammar/nulix.lark`).
//!
//! Эталонные файлы записывают, что проверочный разборщик делает с каждым текстом: принимает
//! или отвергает и какое дерево строит. Разборщик на Rust обязан сделать то же самое.
//! `trees.golden.tsv` — примеры из спецификации, отрицательные примеры и тестовые программы;
//! `fuzz.golden.tsv` — тексты, выведенные из правил грамматики, и они же с одной порчей.

mod common;

use common::unescape;
use nulixc::parser::parse_rule;

/// Сверяет один эталонный файл; возвращает число принятых и число отвергнутых текстов.
fn check(path: &str) -> (usize, usize) {
    let data = std::fs::read_to_string(path).expect("не найден файл с эталонными деревьями");
    let mut accepted = 0;
    let mut rejected = 0;
    let mut failures: Vec<String> = Vec::new();
    for line in data.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert!(fields.len() >= 4, "неверная строка эталона: {}", line);
        let id = fields[0];
        let rule = fields[1];
        let input = unescape(fields[2]);
        let expect_ok = fields[3] == "OK";
        match parse_rule(rule, &input) {
            Ok(tree) => {
                if !expect_ok {
                    failures.push(format!(
                        "{}: проверочный отвергает, этот принял: {}",
                        id, tree
                    ));
                } else if tree != unescape(fields[4]) {
                    failures.push(format!(
                        "{}: деревья разные\n  проверочный: {}\n  этот:        {}",
                        id,
                        unescape(fields[4]),
                        tree
                    ));
                } else {
                    accepted += 1;
                }
            }
            Err(error) => {
                if expect_ok {
                    failures.push(format!(
                        "{}: проверочный принимает, этот отверг: строка {}: {}",
                        id, error.line, error.message
                    ));
                } else {
                    rejected += 1;
                }
            }
        }
    }
    if failures.len() > 15 {
        let total = failures.len();
        failures.truncate(15);
        failures.push(format!("… и ещё {}", total - 15));
    }
    assert!(
        failures.is_empty(),
        "расхождения в {}:\n{}",
        path,
        failures.join("\n")
    );
    (accepted, rejected)
}

fn golden(name: &str) -> String {
    format!("{}/../grammar/{}", env!("CARGO_MANIFEST_DIR"), name)
}

#[test]
fn parser_matches_reference_on_examples() {
    let (accepted, rejected) = check(&golden("trees.golden.tsv"));
    assert!(
        accepted >= 232 && rejected >= 126,
        "в эталоне мало примеров: {} и {}",
        accepted,
        rejected
    );
}

#[test]
fn parser_matches_reference_on_generated_texts() {
    let (accepted, rejected) = check(&golden("fuzz.golden.tsv"));
    assert!(
        accepted >= 1282 && rejected >= 1718,
        "в эталоне мало текстов: {} и {}",
        accepted,
        rejected
    );
}

/// Большой прогон: файл, заданный переменной окружения `NULIX_EXTRA_TREES`, если она есть.
#[test]
fn parser_matches_reference_on_extra_file() {
    if let Ok(path) = std::env::var("NULIX_EXTRA_TREES") {
        let (accepted, rejected) = check(&path);
        println!(
            "дополнительный файл: принято {}, отвергнуто {}",
            accepted, rejected
        );
    }
}
