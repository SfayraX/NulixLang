//! Сверка разборщика с проверочным разборщиком (Lark по `grammar/nulix.lark`).
//!
//! Эталонные файлы записывают, что проверочный разборщик делает с каждым текстом: принимает
//! или отвергает и какое дерево строит. Разборщик на Rust обязан сделать то же самое.
//! `trees.golden.tsv` — примеры из спецификации, отрицательные примеры и тестовые программы;
//! `fuzz.golden.tsv` — тексты, выведенные из правил грамматики, и они же с одной порчей;
//! `depth.golden.tsv` — тексты у предела вложенности: чуть мельче, ровно на пределе и чуть глубже.
//! Там вместо записи дерева стоит её контрольная сумма после «#»: деревья слишком большие.

mod common;

use common::unescape;
use nulixc::parser::parse_rule;

/// Контрольная сумма записи дерева: FNV-1a, 64 бита, как в `tools/grammar/depth_cases.py`.
fn checksum(text: &str) -> String {
    let mut value: u64 = 0xCBF2_9CE4_8422_2325;
    for byte in text.bytes() {
        value = (value ^ byte as u64).wrapping_mul(0x0100_0000_01B3);
    }
    format!("#{:016x}", value)
}

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
                    let shown: String = tree.chars().take(300).collect();
                    failures.push(format!(
                        "{}: проверочный отвергает, этот принял: {}",
                        id, shown
                    ));
                } else if fields[4].starts_with('#') {
                    if checksum(&tree) != fields[4] {
                        failures.push(format!("{}: деревья разные (по контрольной сумме)", id));
                    } else {
                        accepted += 1;
                    }
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
    let (accepted, rejected) = nulixc::stack::run(|| check(&golden("trees.golden.tsv")));
    assert!(
        accepted >= 232 && rejected >= 126,
        "в эталоне мало примеров: {} и {}",
        accepted,
        rejected
    );
}

#[test]
fn parser_matches_reference_on_generated_texts() {
    let (accepted, rejected) = nulixc::stack::run(|| check(&golden("fuzz.golden.tsv")));
    assert!(
        accepted >= 1282 && rejected >= 1718,
        "в эталоне мало текстов: {} и {}",
        accepted,
        rejected
    );
}

#[test]
fn parser_matches_reference_at_nesting_limit() {
    let (accepted, rejected) = nulixc::stack::run(|| check(&golden("depth.golden.tsv")));
    assert!(
        accepted >= 110 && rejected >= 55,
        "в эталоне мало текстов: {} и {}",
        accepted,
        rejected
    );
}

/// Большой прогон: файл, заданный переменной окружения `NULIX_EXTRA_TREES`, если она есть.
#[test]
fn parser_matches_reference_on_extra_file() {
    if let Ok(path) = std::env::var("NULIX_EXTRA_TREES") {
        let (accepted, rejected) = nulixc::stack::run(move || check(&path));
        println!(
            "дополнительный файл: принято {}, отвергнуто {}",
            accepted, rejected
        );
    }
}
