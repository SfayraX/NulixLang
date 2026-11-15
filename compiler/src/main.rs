//! Командная строка начального компилятора. Пока умеет одно: показать лексемы файла.

#![forbid(unsafe_code)]

use nulixc::lexer::lex;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || args[1] != "lex" {
        eprintln!("использование: nulixc lex <файл>");
        std::process::exit(2);
    }
    let source = match std::fs::read_to_string(&args[2]) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("не удалось прочитать {}: {}", args[2], error);
            std::process::exit(2);
        }
    };
    match lex(&source, true) {
        Ok(tokens) => {
            for token in tokens {
                println!("{}\t{}", token.line, token.dump().escape_debug());
            }
        }
        Err(error) => {
            eprintln!("{}:{}: {}", args[2], error.line, error.message);
            std::process::exit(1);
        }
    }
}
