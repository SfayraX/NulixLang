//! Начальный компилятор Nulix.
//!
//! Правила кода: без `unsafe`, без собственных макросов и без внешних зависимостей —
//! чтобы перенос на Nulix был почти построчным. Из готовых макросов используются только
//! стандартные (`format!`, `println!`, `assert!` и подобные) и `derive` для стандартных свойств.

#![forbid(unsafe_code)]

pub mod ast;
pub mod check;
pub mod core;
pub mod dump;
pub mod interp;
pub mod ir;
pub mod ir_text;
pub mod lexer;
pub mod moves;
pub mod parser;
pub mod patterns;
pub mod stack;
pub mod token;
pub mod verify;
