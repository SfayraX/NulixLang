//! Разборщик как его видит остальной компилятор: дерево файла, номера строк и ошибки.

use nulixc::ast::{DeclKind, ExprKind, StmtKind, TopItem};
use nulixc::parser::{parse_file, parse_rule};

#[test]
fn lines_are_recorded() {
    let source = "import a\n\nfunc f() {\n\tlet x = 1\n\treturn x +\n\t\t2\n}\n";
    let file = parse_file(source).expect("текст должен разобраться");
    assert_eq!(file.imports[0].line, 1);
    let decl = match &file.items[0] {
        TopItem::Decl(decl) => decl,
        TopItem::Stmt(_) => panic!("ожидалось объявление"),
    };
    assert_eq!(decl.line, 3);
    let func = match &decl.kind {
        DeclKind::Func(func) => func,
        _ => panic!("ожидалась функция"),
    };
    assert_eq!(func.sig.name, "f");
    assert_eq!(func.body.stmts[0].line, 4);
    assert_eq!(func.body.stmts[1].line, 5);
    let sum = match &func.body.stmts[1].kind {
        StmtKind::Return(Some(value)) => value,
        _ => panic!("ожидался return со значением"),
    };
    match &sum.kind {
        ExprKind::Binary(_, left, right) => {
            assert_eq!(left.line, 5);
            assert_eq!(right.line, 6);
        }
        _ => panic!("ожидалась сумма"),
    }
}

#[test]
fn errors_point_at_the_line() {
    let error = parse_file("let a = 1\nlet b = (2\n\t+ )\n").unwrap_err();
    assert_eq!(error.line, 3);
    assert!(error.message.contains("ожидалось"), "{}", error.message);

    // ошибка лексера доходит с номером строки
    let error = parse_file("let a = 1\nlet b = 007\n").unwrap_err();
    assert_eq!(error.line, 2);

    // ошибка внутри подстановки ${…}: строка самой подстановки, а не начала литерала
    let error = parse_file("let s = \"\"\"\nтекст\n${ 1 + }\n\"\"\"\n").unwrap_err();
    assert_eq!(error.line, 3);
}

#[test]
fn sample_programs_parse() {
    let dir = format!("{}/../grammar/samples", env!("CARGO_MANIFEST_DIR"));
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("нет папки с тестовыми программами")
    {
        let path = entry.expect("не удалось прочитать папку").path();
        let source = std::fs::read_to_string(&path).expect("не удалось прочитать программу");
        if let Err(error) = parse_file(&source) {
            panic!(
                "{}: строка {}: {}",
                path.display(),
                error.line,
                error.message
            );
        }
        count += 1;
    }
    assert_eq!(count, 3);
}

#[test]
fn unknown_rule_is_an_error() {
    assert!(parse_rule("нет_такого_правила", "x").is_err());
}
