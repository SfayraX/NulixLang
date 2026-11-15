//! Лексемы Nulix.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Имя.
    Name,
    /// Целое число.
    Int,
    /// Вещественное число.
    Float,
    /// Строка вместе с кавычками.
    Str,
    /// Символ вместе с кавычками.
    Char,
    /// Пометка вида `@имя`, кроме уровней, строгости и `@file`.
    Attr,
    /// Ключевое слово, а также `@high`, `@strict`, `@file` и им подобные.
    Keyword,
    /// Знак или одиночный `_`.
    Punct,
    /// Конец инструкции: `;` или конец строки.
    Term,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: u32,
}

impl Token {
    /// Запись лексемы для сверки с проверочным лексером.
    pub fn dump(&self) -> String {
        match self.kind {
            TokenKind::Name => format!("NAME:{}", self.text),
            TokenKind::Int => format!("INT:{}", self.text),
            TokenKind::Float => format!("FLOAT:{}", self.text),
            TokenKind::Str => format!("STRING:{}", self.text),
            TokenKind::Char => format!("CHAR:{}", self.text),
            TokenKind::Attr => format!("ATTR:{}", self.text),
            TokenKind::Keyword | TokenKind::Punct => format!("kw:{}", self.text),
            TokenKind::Term => String::from("TERM"),
        }
    }
}
