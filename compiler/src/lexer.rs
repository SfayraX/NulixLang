//! Лексер Nulix.
//!
//! Правила описаны в `spec/grammar.md`, раздел «Лексика». Это перенос проверочного
//! лексера `tools/grammar/nxlex.py`; тест `tests/lexer_golden.rs` сверяет их на всех примерах.
//!
//! Ограничение: имена определяются по свойствам символов из стандартной библиотеки Rust,
//! а не по таблицам Unicode UAX #31. Точные таблицы появятся, когда будет выбрана версия
//! Unicode для первой редакции языка.

use crate::token::{Token, TokenKind};

const KEYWORDS: &[&str] = &[
    "and",
    "any",
    "as",
    "assert",
    "await",
    "break",
    "case",
    "catch",
    "const",
    "continue",
    "discard",
    "else",
    "end",
    "enum",
    "extern",
    "false",
    "for",
    "func",
    "if",
    "impl",
    "import",
    "in",
    "indirect",
    "interface",
    "is",
    "let",
    "life",
    "limit",
    "loop",
    "match",
    "mut",
    "not",
    "or",
    "provide",
    "ptr",
    "pub",
    "ref",
    "return",
    "self",
    "struct",
    "take",
    "target",
    "test",
    "true",
    "trust",
    "try",
    "type",
    "uses",
    "var",
    "while",
    "without",
];

/// Слова, которые являются ключевыми только на своём месте.
const CONTEXTUAL: &[&str] = &["life", "end", "test", "type", "discard"];

/// Ключевые слова, после которых конец строки завершает инструкцию.
const ENDER_WORDS: &[&str] = &[
    "return", "break", "continue", "self", "true", "false", "any", "trust",
];

/// Пометки уровня и строгости и заголовок файла.
const MARKERS: &[&str] = &[
    "@high",
    "@system",
    "@low",
    "@naked",
    "@prototype",
    "@develop",
    "@production",
    "@strict",
    "@file",
];

/// Знаки, от длинных к коротким.
const OPERATORS: &[&str] = &[
    "<<=", ">>=", "..<", "..=", "->", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "&=",
    "|=", "^=", "<<", ">>", "..", "(", ")", "[", "]", "{", "}", ",", ":", ".", "=", "<", ">", "+",
    "-", "*", "/", "%", "&", "|", "^", "~",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub line: u32,
    pub message: String,
}

/// Разбивает текст на лексемы. `eof_term` — ставить ли конец инструкции в конце текста,
/// как при разборе целого файла.
pub fn lex(source: &str, eof_term: bool) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer {
        text: source.chars().collect(),
        line: 1,
        out: Vec::new(),
        stack: Vec::new(),
    };
    lexer.run(eof_term)?;
    Ok(lexer.out)
}

fn ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn ident_cont(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn is_digit_or_underscore(c: char) -> bool {
    c.is_ascii_digit() || c == '_'
}

struct Lexer {
    text: Vec<char>,
    line: u32,
    out: Vec<Token>,
    /// Открытые скобки: `(`, `[` или `{`.
    stack: Vec<char>,
}

impl Lexer {
    /// Символ в позиции `i` или `\0` за концом текста.
    fn ch(&self, i: usize) -> char {
        if i < self.text.len() {
            self.text[i]
        } else {
            '\0'
        }
    }

    fn starts_with(&self, i: usize, s: &str) -> bool {
        let mut k = i;
        for c in s.chars() {
            if k >= self.text.len() || self.text[k] != c {
                return false;
            }
            k += 1;
        }
        true
    }

    fn slice(&self, from: usize, to: usize) -> String {
        self.text[from..to].iter().collect()
    }

    fn error(&self, message: &str) -> LexError {
        LexError {
            line: self.line,
            message: String::from(message),
        }
    }

    fn emit(&mut self, kind: TokenKind, text: String) {
        self.out.push(Token {
            kind,
            text,
            line: self.line,
        });
    }

    /// Завершает ли инструкцию конец строки после последней лексемы.
    fn ends_statement(&self) -> bool {
        let n = self.out.len();
        if n == 0 {
            return false;
        }
        let last = &self.out[n - 1];
        if last.kind == TokenKind::Term {
            return false;
        }
        if n > 1 {
            let before = &self.out[n - 2];
            if before.kind == TokenKind::Keyword && before.text == "end" {
                return true; // лексема после end — это метка конца блока
            }
        }
        match last.kind {
            TokenKind::Name
            | TokenKind::Int
            | TokenKind::Float
            | TokenKind::Str
            | TokenKind::Char => true,
            TokenKind::Punct => {
                last.text == ")" || last.text == "]" || last.text == "}" || last.text == "_"
            }
            TokenKind::Keyword => ENDER_WORDS.contains(&last.text.as_str()),
            TokenKind::Attr | TokenKind::Term => false,
        }
    }

    /// Пропускает пробелы, переводы строк и комментарии.
    fn skip_blank(&self, from: usize) -> usize {
        let n = self.text.len();
        let mut k = from;
        while k < n {
            let c = self.text[k];
            if c == ' ' || c == '\t' || c == '\r' || c == '\n' {
                k += 1;
            } else if self.starts_with(k, "//") {
                while k < n && self.text[k] != '\n' {
                    k += 1;
                }
            } else if self.starts_with(k, "/*") {
                let mut depth = 1;
                k += 2;
                while k < n && depth > 0 {
                    if self.starts_with(k, "/*") {
                        depth += 1;
                        k += 2;
                    } else if self.starts_with(k, "*/") {
                        depth -= 1;
                        k += 2;
                    } else {
                        k += 1;
                    }
                }
            } else {
                break;
            }
        }
        k
    }

    /// Конец строки: ставит конец инструкции, если он здесь нужен.
    fn newline(&mut self, next: usize) {
        if let Some(&top) = self.stack.last() {
            if top == '(' || top == '[' {
                return; // внутри круглых и квадратных скобок перевод строки ничего не завершает
            }
        }
        let k = self.skip_blank(next);
        if self.starts_with(k, ".") && !self.starts_with(k, "..") {
            return; // строка, начинающаяся с точки, продолжает предыдущую
        }
        if self.ends_statement() {
            self.emit(TokenKind::Term, String::from("\n"));
        }
    }

    /// Ключевое ли здесь слово из `CONTEXTUAL`; `k` — позиция первого знака после слова и пробелов.
    fn contextual_keyword(&self, word: &str, k: usize) -> bool {
        let after = self.ch(k);
        let mut j = k;
        while j < self.text.len() && ident_cont(self.text[j]) {
            j += 1;
        }
        let next_word = self.slice(k, j);
        let prev = self.out.last();
        if word == "life" {
            // после "[" или "," и перед именем или "("
            return match prev {
                Some(p) => {
                    p.kind == TokenKind::Punct
                        && (p.text == "[" || p.text == ",")
                        && (after == '(' || !next_word.is_empty())
                }
                None => false,
            };
        }
        if word == "end" {
            // только сразу после "}"
            return match prev {
                Some(p) => p.kind == TokenKind::Punct && p.text == "}",
                None => false,
            };
        }
        if word == "test" {
            return after == '"'; // только перед строкой
        }
        if word == "type" {
            // только перед именем
            return !next_word.is_empty()
                && ident_start(after)
                && (!KEYWORDS.contains(&next_word.as_str())
                    || CONTEXTUAL.contains(&next_word.as_str()));
        }
        next_word == "self" // discard — только перед self
    }

    /// Конец экранирования, которое начинается в позиции `i` с обратной черты.
    fn match_escape(&self, i: usize) -> Option<usize> {
        if self.ch(i) != '\\' {
            return None;
        }
        let c = self.ch(i + 1);
        if c == 'n'
            || c == 't'
            || c == 'r'
            || c == '0'
            || c == '\\'
            || c == '"'
            || c == '\''
            || c == '$'
        {
            return Some(i + 2);
        }
        if c == 'u' && self.ch(i + 2) == '{' {
            let mut k = i + 3;
            while self.ch(k).is_ascii_hexdigit() {
                k += 1;
            }
            let digits = k - (i + 3);
            if digits >= 1 && digits <= 6 && self.ch(k) == '}' {
                return Some(k + 1);
            }
        }
        None
    }

    /// Проверяет экранирование и подстановки внутри строки, от `from` до `to`.
    fn scan_string_body(&self, from: usize, to: usize) -> Result<(), LexError> {
        let mut i = from;
        while i < to {
            let c = self.text[i];
            if c == '\\' {
                match self.match_escape(i) {
                    Some(end) if end <= to => i = end,
                    _ => return Err(self.error("неизвестное экранирование в строке")),
                }
            } else if c == '$' && i + 1 < to && self.text[i + 1] == '{' {
                let mut depth = 1;
                let mut j = i + 2;
                while j < to && depth > 0 {
                    if self.text[j] == '{' {
                        depth += 1;
                    } else if self.text[j] == '}' {
                        depth -= 1;
                    }
                    j += 1;
                }
                if depth > 0 {
                    return Err(self.error("не закрыта подстановка ${…}"));
                }
                i = j;
            } else if c == '$' && i + 1 < to && ident_start(self.text[i + 1]) {
                i += 2;
                while i < to && ident_cont(self.text[i]) {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    /// Конец экспоненты, которая начинается в позиции `k`, или 0, если её там нет.
    fn scan_exponent(&self, k: usize) -> usize {
        let c = self.ch(k);
        if c != 'e' && c != 'E' {
            return 0;
        }
        let mut m = k + 1;
        if self.ch(m) == '+' || self.ch(m) == '-' {
            m += 1;
        }
        if !self.ch(m).is_ascii_digit() {
            return 0;
        }
        while self.ch(m).is_ascii_digit() {
            m += 1;
        }
        m
    }

    /// Число, которое начинается в позиции `i`: его конец и признак вещественного.
    fn scan_number(&self, i: usize) -> (usize, bool) {
        let mut j = i + 1;
        if self.text[i] != '0' {
            while is_digit_or_underscore(self.ch(j)) {
                j += 1;
            }
        }
        if self.ch(j) == '.' && self.ch(j + 1).is_ascii_digit() {
            let mut k = j + 2;
            while is_digit_or_underscore(self.ch(k)) {
                k += 1;
            }
            let e = self.scan_exponent(k);
            return (if e > 0 { e } else { k }, true);
        }
        let e = self.scan_exponent(j);
        if e > 0 {
            return (e, true);
        }
        if self.text[i] == '0' {
            let prefix = self.ch(i + 1);
            let digit = self.ch(i + 2);
            if prefix == 'x' && digit.is_ascii_hexdigit() {
                let mut k = i + 3;
                while self.ch(k).is_ascii_hexdigit() || self.ch(k) == '_' {
                    k += 1;
                }
                return (k, false);
            }
            if prefix == 'b' && (digit == '0' || digit == '1') {
                let mut k = i + 3;
                while self.ch(k) == '0' || self.ch(k) == '1' || self.ch(k) == '_' {
                    k += 1;
                }
                return (k, false);
            }
            if prefix == 'o' && digit >= '0' && digit <= '7' {
                let mut k = i + 3;
                while (self.ch(k) >= '0' && self.ch(k) <= '7') || self.ch(k) == '_' {
                    k += 1;
                }
                return (k, false);
            }
            return (i + 1, false);
        }
        (j, false)
    }

    fn run(&mut self, eof_term: bool) -> Result<(), LexError> {
        let n = self.text.len();
        let mut i = 0;
        while i < n {
            let c = self.text[i];
            if c == '\n' {
                self.newline(i + 1);
                self.line += 1;
                i += 1;
            } else if c == ' ' || c == '\t' || c == '\r' {
                i += 1;
            } else if self.starts_with(i, "//") {
                while i < n && self.text[i] != '\n' {
                    i += 1;
                }
            } else if self.starts_with(i, "/*") {
                let mut depth = 1;
                i += 2;
                while i < n && depth > 0 {
                    if self.starts_with(i, "/*") {
                        depth += 1;
                        i += 2;
                    } else if self.starts_with(i, "*/") {
                        depth -= 1;
                        i += 2;
                    } else {
                        if self.text[i] == '\n' {
                            self.line += 1;
                        }
                        i += 1;
                    }
                }
                if depth > 0 {
                    return Err(self.error("не закрыт комментарий /* */"));
                }
            } else if c == ';' {
                let after_term = match self.out.last() {
                    Some(t) => t.kind == TokenKind::Term,
                    None => true,
                };
                if !after_term {
                    self.emit(TokenKind::Term, String::from(";"));
                }
                i += 1;
            } else if self.starts_with(i, "\"\"\"") {
                let mut j = i + 3;
                while j < n && !self.starts_with(j, "\"\"\"") {
                    j += 1;
                }
                if j >= n {
                    return Err(self.error("не закрыта многострочная строка"));
                }
                self.scan_string_body(i + 3, j)?;
                let text = self.slice(i, j + 3);
                self.emit(TokenKind::Str, text);
                let mut k = i;
                while k < j {
                    if self.text[k] == '\n' {
                        self.line += 1;
                    }
                    k += 1;
                }
                i = j + 3;
            } else if c == '"' {
                let mut j = i + 1;
                while j < n && self.text[j] != '"' {
                    if self.text[j] == '\n' {
                        return Err(self.error("перевод строки внутри строки в кавычках"));
                    }
                    j += if self.text[j] == '\\' { 2 } else { 1 };
                }
                if j >= n {
                    return Err(self.error("не закрыта строка"));
                }
                self.scan_string_body(i + 1, j)?;
                let text = self.slice(i, j + 1);
                self.emit(TokenKind::Str, text);
                i = j + 1;
            } else if c == '\'' {
                let first = self.ch(i + 1);
                let j = match self.match_escape(i + 1) {
                    Some(end) => end,
                    None => i + 2,
                };
                if j >= n || self.text[j] != '\'' || first == '\'' || first == '\n' {
                    return Err(self.error("неверный символьный литерал"));
                }
                let text = self.slice(i, j + 1);
                self.emit(TokenKind::Char, text);
                i = j + 1;
            } else if c.is_ascii_digit() {
                if c == '0' && self.ch(i + 1).is_ascii_digit() {
                    return Err(self.error("ведущий ноль в числе"));
                }
                let (end, is_float) = self.scan_number(i);
                if end < n && ident_cont(self.text[end]) {
                    return Err(self.error("буква сразу после числа"));
                }
                let text = self.slice(i, end);
                self.emit(
                    if is_float {
                        TokenKind::Float
                    } else {
                        TokenKind::Int
                    },
                    text,
                );
                i = end;
            } else if c == '@' {
                let mut j = i + 1;
                while j < n && ident_cont(self.text[j]) {
                    j += 1;
                }
                if j == i + 1 {
                    return Err(self.error("после @ нужно имя пометки"));
                }
                let word = self.slice(i, j);
                let kind = if MARKERS.contains(&word.as_str()) {
                    TokenKind::Keyword
                } else {
                    TokenKind::Attr
                };
                self.emit(kind, word);
                i = j;
            } else if ident_start(c) {
                let mut j = i + 1;
                while j < n && ident_cont(self.text[j]) {
                    j += 1;
                }
                let word = self.slice(i, j);
                i = j;
                let mut k = i;
                while k < n && (self.text[k] == ' ' || self.text[k] == '\t') {
                    k += 1;
                }
                let after_dot_or_func = match self.out.last() {
                    Some(p) => {
                        (p.kind == TokenKind::Punct && p.text == ".")
                            || (p.kind == TokenKind::Keyword && p.text == "func")
                    }
                    None => false,
                };
                let kind = if word == "_" {
                    TokenKind::Punct
                } else if !KEYWORDS.contains(&word.as_str()) {
                    TokenKind::Name
                } else if after_dot_or_func {
                    TokenKind::Name // после точки и после func любое слово — имя
                } else if CONTEXTUAL.contains(&word.as_str()) {
                    if self.contextual_keyword(&word, k) {
                        TokenKind::Keyword
                    } else {
                        TokenKind::Name
                    }
                } else {
                    TokenKind::Keyword
                };
                self.emit(kind, word);
            } else {
                let mut found = "";
                for &op in OPERATORS {
                    if self.starts_with(i, op) {
                        found = op;
                        break;
                    }
                }
                if found.is_empty() {
                    return Err(self.error("неизвестный знак"));
                }
                if found == "(" || found == "[" || found == "{" {
                    self.stack.push(c);
                } else if found == ")" || found == "]" || found == "}" {
                    let open = if found == ")" {
                        '('
                    } else if found == "]" {
                        '['
                    } else {
                        '{'
                    };
                    match self.stack.last() {
                        Some(&top) if top == open => {
                            self.stack.pop();
                        }
                        _ => return Err(self.error("лишняя или непарная скобка")),
                    }
                }
                self.emit(TokenKind::Punct, String::from(found));
                i += found.chars().count();
            }
        }
        if !self.stack.is_empty() {
            return Err(self.error("не закрыта скобка"));
        }
        if eof_term && self.ends_statement() {
            self.emit(TokenKind::Term, String::new());
        }
        Ok(())
    }
}
