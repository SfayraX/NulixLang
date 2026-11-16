//! Командная строка начального компилятора: показать лексемы файла или его дерево разбора.

#![forbid(unsafe_code)]

use nulixc::dump;
use nulixc::lexer::lex;
use nulixc::parser::parse_file;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || (args[1] != "lex" && args[1] != "parse") {
        eprintln!("использование: nulixc lex <файл>");
        eprintln!("               nulixc parse <файл>");
        std::process::exit(2);
    }
    let source = match std::fs::read_to_string(&args[2]) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("не удалось прочитать {}: {}", args[2], error);
            std::process::exit(2);
        }
    };
    if args[1] == "lex" {
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
        return;
    }
    match parse_file(&source) {
        Ok(file) => {
            // по одной записи на строку: заголовок, импорты, объявления и инструкции
            if let Some(header) = &file.header {
                println!("{}", dump::header(header));
            }
            for item in &file.imports {
                println!("{}", dump::import(item));
            }
            for item in &file.items {
                match item {
                    nulixc::ast::TopItem::Decl(d) => println!("{}", dump::decl(d)),
                    nulixc::ast::TopItem::Stmt(s) => println!("{}", dump::stmt(s)),
                }
            }
        }
        Err(error) => {
            eprintln!("{}:{}: {}", args[2], error.line, error.message);
            std::process::exit(1);
        }
    }
}
