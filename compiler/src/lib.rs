//! Начальный компилятор Nulix.
//!
//! Правила кода: без `unsafe`, без собственных макросов и без внешних зависимостей —
//! чтобы перенос на Nulix был почти построчным. Из готовых макросов используются только
//! стандартные (`format!`, `println!`, `assert!` и подобные) и `derive` для стандартных свойств.

#![forbid(unsafe_code)]

pub mod ast;
pub mod dump;
pub mod lexer;
pub mod parser;
pub mod stack;
pub mod token;
