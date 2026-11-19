//! Сверка лексера с проверочным лексером на Python.
//!
//! Файл `grammar/tokens.golden.tsv` записывает, что выдаёт проверочный лексер на каждом
//! примере. Лексер на Rust обязан выдать то же самое, а где тот сообщает об ошибке — тоже ошибку.

mod common;

use common::unescape;
use nulixc::lexer::lex;

#[test]
fn lexer_matches_reference() {
    let path = format!(
        "{}/../grammar/tokens.golden.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read_to_string(&path).expect("не найден файл с эталонными лексемами");
    let mut checked = 0;
    let mut errors_expected = 0;
    let mut failures: Vec<String> = Vec::new();
    for line in data.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert!(fields.len() >= 4, "неверная строка эталона: {}", line);
        let id = fields[0];
        let eof_term = fields[1] == "1";
        let input = unescape(fields[2]);
        let expect_ok = fields[3] == "OK";
        let mut expected: Vec<String> = Vec::new();
        for field in &fields[4..] {
            expected.push(unescape(field));
        }
        checked += 1;
        match lex(&input, eof_term) {
            Ok(tokens) => {
                let mut actual: Vec<String> = Vec::new();
                for token in &tokens {
                    actual.push(token.dump());
                }
                if !expect_ok {
                    failures.push(format!(
                        "{}: ожидалась ошибка, получены лексемы {:?}",
                        id, actual
                    ));
                } else if actual != expected {
                    failures.push(format!(
                        "{}: ожидалось {:?}, получено {:?}",
                        id, expected, actual
                    ));
                }
            }
            Err(error) => {
                if expect_ok {
                    failures.push(format!(
                        "{}: неожиданная ошибка в строке {}: {}",
                        id, error.line, error.message
                    ));
                } else {
                    errors_expected += 1;
                }
            }
        }
    }
    assert!(
        checked > 200,
        "в эталоне слишком мало примеров: {}",
        checked
    );
    assert!(
        errors_expected > 0,
        "в эталоне нет ни одного примера с ошибкой"
    );
    assert!(
        failures.is_empty(),
        "расхождений: {}\n{}",
        failures.len(),
        failures.join("\n")
    );
}
