//! Проверенное дерево явного ядра — то, что проверка исходника (`check.rs`) отдаёт переводу в IR.
//!
//! Правила — `spec/core-check.md`. Всё, что лежит здесь, уже проверено: имена объявлений
//! разрешены, типы полей и параметров известны, простые типы отмечены. Пока в дереве объявления
//! и сигнатуры функций; тела функций и тестов появятся в нём вместе со своей проверкой
//! (`spec/core-check.md`, раздел 9).

use crate::ir::{Mode, Scalar};

/// Номер записи или перечисления в `Program::types`.
pub type TypeId = usize;

/// Тип явного ядра шага 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Scalar(Scalar),
    /// Единичный тип `()`.
    Unit,
    /// Кортеж, не меньше двух элементов.
    Tuple(Vec<Type>),
    /// Массив `Array[T, N]`.
    Array(Box<Type>, u64),
    /// Запись или перечисление пакета.
    Named(TypeId),
    /// Встроенный `Never` — перечисление без вариантов (9.36).
    Never,
}

/// Проверенная программа: записи и перечисления, функции и тесты в порядке объявления.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    pub types: Vec<TypeDecl>,
    pub funcs: Vec<Func>,
    pub tests: Vec<Test>,
}

/// Запись или перечисление.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub name: String,
    /// Тип простой: для него объявлено `impl Copy for T {}` (12.27).
    pub copy: bool,
    pub kind: TypeKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Struct(Vec<Field>),
    Enum(Vec<Variant>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub payload: Payload,
    pub line: u32,
}

/// Что несёт вариант: ничего, одно безымянное значение или именованные поля.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    None,
    One(Type),
    Fields(Vec<Field>),
}

/// Функция. У функции без `->` результат — `()` (12.15).
#[derive(Debug, Clone, PartialEq)]
pub struct Func {
    pub name: String,
    pub params: Vec<Param>,
    pub result: Type,
    pub line: u32,
}

/// Параметр: без слова — чтение, `mut` — изменение на месте, `take` — передача владения (12.58).
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub mode: Mode,
    pub line: u32,
}

/// Тест `test "…" { … }`; имя — значение строки.
#[derive(Debug, Clone, PartialEq)]
pub struct Test {
    pub name: String,
    pub line: u32,
}

impl Program {
    /// Простой тип: там, где непростое значение переместилось бы, простое копируется
    /// (10.2, 12.27). `Never` простой: значений у него нет.
    pub fn is_simple(&self, ty: &Type) -> bool {
        match ty {
            Type::Scalar(_) | Type::Unit | Type::Never => true,
            Type::Tuple(items) => items.iter().all(|item| self.is_simple(item)),
            Type::Array(item, _) => self.is_simple(item),
            Type::Named(id) => self.types[*id].copy,
        }
    }

    /// Тип без значений: `Never` и любое перечисление без вариантов (9.36).
    pub fn is_never(&self, ty: &Type) -> bool {
        match ty {
            Type::Never => true,
            Type::Named(id) => {
                matches!(&self.types[*id].kind, TypeKind::Enum(variants) if variants.is_empty())
            }
            _ => false,
        }
    }

    /// Запись типа так, как её пишут в исходнике.
    pub fn type_text(&self, ty: &Type) -> String {
        match ty {
            Type::Scalar(s) => s.name().to_string(),
            Type::Unit => "()".to_string(),
            Type::Tuple(items) => {
                let parts: Vec<String> = items.iter().map(|item| self.type_text(item)).collect();
                format!("({})", parts.join(", "))
            }
            Type::Array(item, n) => format!("Array[{}, {}]", self.type_text(item), n),
            Type::Named(id) => self.types[*id].name.clone(),
            Type::Never => "Never".to_string(),
        }
    }
}
