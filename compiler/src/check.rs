//! Проверка явного ядра в исходнике: дерево разбора превращается в проверенное дерево `core`.
//!
//! Правила — `spec/core-check.md`. Проверка останавливается на первой ошибке и называет её
//! строку. Она не зависит ни от проверщика IR, ни от интерпретатора.
//!
//! Сейчас проверяются файл, объявления и сигнатуры — шаг 1 раздела 9 `spec/core-check.md`:
//! заголовок `@file`, конструкции вне шага 2, имена объявлений, типы полей, вариантов,
//! параметров и результатов, `impl Copy for T {}`, типы, которые содержат сами себя, метки `end`
//! у объявлений и имена тестов. Тела функций и тестов пока не проверяются.
//!
//! Порядок: сначала все объявления по порядку файла — их форма и имена; затем типы полей,
//! вариантов, параметров и результатов; затем `impl Copy`; затем типы, которые содержат сами
//! себя; затем простота частей у типов с `impl Copy`. Рекурсия идёт только по вложенности типов,
//! а её ограничивает предел вложенности разборщика; вызывать проверку, как и разбор, нужно
//! через `stack::run`.

use crate::ast;
use crate::core::{
    Field, Func, Param, Payload, Program, Test, Type, TypeDecl, TypeId, TypeKind, Variant,
};
use crate::ir::{Mode, Scalar};
use std::collections::{HashMap, HashSet};

/// Ошибка проверки: строка и сообщение.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError {
    pub line: u32,
    pub message: String,
}

type R<T> = Result<T, CheckError>;

fn fail<T>(line: u32, message: String) -> R<T> {
    Err(CheckError { line, message })
}

/// Конструкция, которой нет в шаге 2 (`spec/core-check.md`, раздел 3).
fn outside<T>(line: u32, what: &str) -> R<T> {
    fail(line, format!("не входит в шаг 2: {}", what))
}

/// Имена, доступные без импорта (12.89–12.91): своим объявлением их не называют (решение 17).
const STANDARD_NAMES: [&str; 23] = [
    "Option", "Result", "Some", "None", "Ok", "Err", "print", "Drop", "Copy", "From", "Error",
    "Display", "Debug", "Eq", "Ord", "Hash", "Iterator", "List", "Str", "Bytes", "TextOut",
    "size_of", "align_of",
];

/// Слова строгости в заголовке `@file(…)` (12.66, 12.68).
const STRICTNESS: [&str; 4] = ["prototype", "develop", "production", "strict"];

/// Встроенный тип: скаляр, `Never` или `Array` (12.19, 9.36, 12.37). Своим объявлением его
/// не называют (решение 10и).
fn builtin_type(name: &str) -> bool {
    Scalar::from_name(name).is_some() || name == "Never" || name == "Array"
}

/// Проверяет файл и отдаёт проверенную программу или первую ошибку.
pub fn check(file: &ast::File) -> R<Program> {
    let mut c = Checker::default();
    c.header(file)?;
    if let Some(import) = file.imports.first() {
        return outside(import.line, "импорт");
    }
    for item in &file.items {
        match item {
            ast::TopItem::Stmt(stmt) => {
                return outside(stmt.line, "инструкции на верхнем уровне");
            }
            ast::TopItem::Decl(decl) => c.decl(decl)?,
        }
    }
    c.resolve()?;
    c.copy_impls()?;
    c.self_containing()?;
    c.copy_parts()?;
    Ok(c.program)
}

/// Что объявлено под именем пакета.
#[derive(Debug, Clone, Copy)]
enum Item {
    Type(TypeId),
    Func,
}

/// Запись или перечисление в дереве разбора.
#[derive(Debug, Clone, Copy)]
enum TypeSource<'a> {
    Struct(&'a ast::StructDecl),
    Enum(&'a ast::EnumDecl),
}

#[derive(Default)]
struct Checker<'a> {
    program: Program,
    /// Имена объявлений пакета — функций и типов, у них одно пространство имён: что объявлено
    /// и в какой строке.
    names: HashMap<&'a str, (Item, u32)>,
    /// Записи и перечисления из дерева разбора, по номерам в `program.types`.
    types: Vec<TypeSource<'a>>,
    /// Сигнатуры функций из дерева разбора, по номерам в `program.funcs`.
    funcs: Vec<&'a ast::FuncSig>,
    /// `impl Copy for T {}`: тип после `for` и строка объявления.
    impls: Vec<(&'a ast::PathType, u32)>,
    /// Строка `impl Copy` у каждого простого типа.
    copy_lines: HashMap<TypeId, u32>,
    /// Имена тестов и строки, где они объявлены.
    tests: HashMap<String, u32>,
}

impl<'a> Checker<'a> {
    // ───────── файл и объявления ─────────

    /// Заголовок `@file(строгость)`: на проверку в шаге 2 он не влияет (решение 10м).
    fn header(&mut self, file: &ast::File) -> R<()> {
        let Some(args) = &file.header else {
            return Ok(());
        };
        let line = file.header_line;
        let mut strictness = false;
        for arg in args {
            match arg {
                ast::HeaderArg::Target(_) => {
                    return outside(line, "target(…) в заголовке @file");
                }
                ast::HeaderArg::Name(name) => {
                    if !STRICTNESS.contains(&name.as_str()) {
                        return fail(line, format!("@file: неизвестная строгость {}", name));
                    }
                    if strictness {
                        return fail(line, "@file: строгость указана дважды".to_string());
                    }
                    strictness = true;
                }
            }
        }
        Ok(())
    }

    fn decl(&mut self, decl: &'a ast::Decl) -> R<()> {
        for attr in &decl.attrs {
            match attr {
                ast::Attr::Level(level) if level == "@high" || level == "@system" => {}
                ast::Attr::Level(level) => return outside(decl.line, level),
                ast::Attr::Strictness(marker) => {
                    return outside(
                        decl.line,
                        &format!("пометка строгости {} у объявления", marker),
                    );
                }
                ast::Attr::Custom(name, _) => {
                    return outside(decl.line, &format!("пометка {}", name));
                }
            }
        }
        match &decl.kind {
            ast::DeclKind::Binding(binding) => {
                let word = match binding.kind {
                    ast::BindKind::Let => "let",
                    ast::BindKind::Var => "var",
                    ast::BindKind::Const => "const",
                };
                outside(binding.line, &format!("{} на верхнем уровне", word))
            }
            ast::DeclKind::Func(func) => self.func_decl(func),
            ast::DeclKind::ExternBlock(_) => outside(decl.line, "extern"),
            ast::DeclKind::Struct(s) => self.struct_decl(s, decl.line),
            ast::DeclKind::Enum(e) => self.enum_decl(e, decl.line),
            ast::DeclKind::Interface(_) => outside(decl.line, "interface"),
            ast::DeclKind::Impl(i) => self.impl_decl(i, decl.line),
            ast::DeclKind::TypeAlias(_) => outside(decl.line, "type"),
            ast::DeclKind::Test(t) => self.test_decl(t, decl.line),
        }
    }

    fn func_decl(&mut self, func: &'a ast::FuncDecl) -> R<()> {
        let sig = &func.sig;
        if func.abi.is_some() {
            return outside(sig.line, "extern");
        }
        if !sig.generics.is_empty() {
            return outside(sig.line, "обобщения");
        }
        if let Some(reqs) = &sig.reqs {
            let word = if reqs.uses.is_some() {
                "uses"
            } else if reqs.without.is_some() {
                "without"
            } else {
                "limit"
            };
            return outside(sig.line, word);
        }
        let mut seen = HashSet::new();
        for param in &sig.params {
            match &param.kind {
                ast::ParamKind::SelfValue => return outside(param.line, "методы и self"),
                ast::ParamKind::Named(name, ty) => {
                    if ty.is_none() {
                        return fail(param.line, format!("параметр {}: тип не указан", name));
                    }
                    if !seen.insert(name.as_str()) {
                        return fail(param.line, format!("параметр {} повторён", name));
                    }
                }
            }
        }
        if let Some(label) = &func.end {
            if *label != sig.name {
                return fail(
                    sig.line,
                    format!(
                        "метка end {} не совпадает с именем функции {}",
                        label, sig.name
                    ),
                );
            }
        }
        self.declare(&sig.name, sig.line, Item::Func)?;
        self.funcs.push(sig);
        Ok(())
    }

    fn struct_decl(&mut self, s: &'a ast::StructDecl, line: u32) -> R<()> {
        if !s.generics.is_empty() {
            return outside(line, "обобщения");
        }
        let mut seen = HashSet::new();
        for field in &s.fields {
            if !field.attrs.is_empty() {
                return outside(field.line, "пометки полей");
            }
            if field.default.is_some() {
                return outside(field.line, "значения полей по умолчанию");
            }
            if !seen.insert(field.name.as_str()) {
                return fail(field.line, format!("поле {} повторено", field.name));
            }
        }
        if let Some(label) = &s.end {
            if *label != s.name {
                return fail(
                    line,
                    format!(
                        "метка end {} не совпадает с именем записи {}",
                        label, s.name
                    ),
                );
            }
        }
        self.declare_type(&s.name, line, TypeSource::Struct(s))
    }

    fn enum_decl(&mut self, e: &'a ast::EnumDecl, line: u32) -> R<()> {
        if !e.generics.is_empty() {
            return outside(line, "обобщения");
        }
        let mut seen = HashSet::new();
        for variant in &e.variants {
            if variant.indirect {
                return outside(variant.line, "indirect");
            }
            if !seen.insert(variant.name.as_str()) {
                return fail(variant.line, format!("вариант {} повторён", variant.name));
            }
            if let ast::VariantFields::Named(fields) = &variant.fields {
                let mut names = HashSet::new();
                for field in fields {
                    if !names.insert(field.name.as_str()) {
                        return fail(
                            variant.line,
                            format!("поле {} варианта {} повторено", field.name, variant.name),
                        );
                    }
                }
            }
        }
        if let Some(label) = &e.end {
            if *label != e.name {
                return fail(
                    line,
                    format!(
                        "метка end {} не совпадает с именем перечисления {}",
                        label, e.name
                    ),
                );
            }
        }
        self.declare_type(&e.name, line, TypeSource::Enum(e))
    }

    /// Из реализаций в шаге 2 есть только `impl Copy for T {}` (12.27). Тип после `for`
    /// разрешается, когда объявлены все имена: порядок объявлений в файле не важен.
    fn impl_decl(&mut self, i: &'a ast::ImplDecl, line: u32) -> R<()> {
        let target = match &i.for_type {
            Some(target)
                if i.generics.is_empty()
                    && i.first.names == ["Copy"]
                    && i.first.args.is_empty() =>
            {
                target
            }
            _ => return outside(line, "impl, кроме impl Copy for T {}"),
        };
        let text = path_text(target);
        if !i.members.is_empty() {
            return fail(
                line,
                format!("impl Copy for {} пишется пустым (12.27)", text),
            );
        }
        if let Some(end) = &i.end {
            if end.first != i.first.names || end.for_type.as_ref() != Some(&target.names) {
                let mut label = format!("end impl {}", end.first.join("."));
                if let Some(names) = &end.for_type {
                    label = format!("{} for {}", label, names.join("."));
                }
                return fail(
                    line,
                    format!(
                        "метка {} не совпадает с заголовком impl Copy for {}",
                        label, text
                    ),
                );
            }
        }
        self.impls.push((target, line));
        Ok(())
    }

    fn test_decl(&mut self, t: &'a ast::TestDecl, line: u32) -> R<()> {
        let name = &t.name;
        if !name.subs.is_empty() {
            return fail(name.line, "имя теста — строка без подстановок".to_string());
        }
        if name.raw.starts_with("\"\"\"") {
            return outside(name.line, "многострочная строка в имени теста");
        }
        let value = match decode_string(&name.raw) {
            Ok(value) => value,
            Err(message) => return fail(name.line, message),
        };
        if let Some(first) = self.tests.get(&value) {
            return fail(
                name.line,
                format!("тест «{}» уже объявлен в строке {}", value, first),
            );
        }
        self.tests.insert(value.clone(), name.line);
        self.program.tests.push(Test { name: value, line });
        Ok(())
    }

    /// Имя объявления пакета: не встроенный тип, не стандартное имя и не повтор.
    fn declare(&mut self, name: &'a str, line: u32, item: Item) -> R<()> {
        if builtin_type(name) {
            return fail(line, format!("имя {} занято встроенным типом", name));
        }
        if STANDARD_NAMES.contains(&name) {
            return fail(
                line,
                format!(
                    "имя {} занято: оно доступно без импорта (12.89–12.91)",
                    name
                ),
            );
        }
        if let Some((_, first)) = self.names.get(name) {
            return fail(
                line,
                format!("имя {} уже объявлено в строке {}", name, first),
            );
        }
        self.names.insert(name, (item, line));
        Ok(())
    }

    fn declare_type(&mut self, name: &'a str, line: u32, source: TypeSource<'a>) -> R<()> {
        let id = self.program.types.len();
        self.declare(name, line, Item::Type(id))?;
        // части типа разрешаются в `resolve`, когда объявлены все имена
        let kind = match source {
            TypeSource::Struct(_) => TypeKind::Struct(Vec::new()),
            TypeSource::Enum(_) => TypeKind::Enum(Vec::new()),
        };
        self.program.types.push(TypeDecl {
            name: name.to_string(),
            copy: false,
            kind,
            line,
        });
        self.types.push(source);
        Ok(())
    }

    // ───────── типы ─────────

    /// Типы полей, вариантов, параметров и результатов.
    fn resolve(&mut self) -> R<()> {
        for id in 0..self.types.len() {
            let kind = match self.types[id] {
                TypeSource::Struct(s) => TypeKind::Struct(self.fields(&s.fields)?),
                TypeSource::Enum(e) => {
                    let mut variants = Vec::new();
                    for variant in &e.variants {
                        let payload = match &variant.fields {
                            ast::VariantFields::None => Payload::None,
                            ast::VariantFields::Positional(ty) => Payload::One(self.ty(ty)?),
                            ast::VariantFields::Named(fields) => {
                                let mut out = Vec::new();
                                for field in fields {
                                    out.push(Field {
                                        name: field.name.clone(),
                                        ty: self.ty(&field.ty)?,
                                        line: variant.line,
                                    });
                                }
                                Payload::Fields(out)
                            }
                        };
                        variants.push(Variant {
                            name: variant.name.clone(),
                            payload,
                            line: variant.line,
                        });
                    }
                    TypeKind::Enum(variants)
                }
            };
            self.program.types[id].kind = kind;
        }
        for k in 0..self.funcs.len() {
            let sig = self.funcs[k];
            let mut params = Vec::new();
            for param in &sig.params {
                // `self` и параметры без типа отвергнуты в `func_decl`
                if let ast::ParamKind::Named(name, Some(ty)) = &param.kind {
                    params.push(Param {
                        name: name.clone(),
                        ty: self.ty(ty)?,
                        mode: match param.mode {
                            None => Mode::Read,
                            Some(ast::Mode::Mut) => Mode::Mut,
                            Some(ast::Mode::Take) => Mode::Take,
                        },
                        line: param.line,
                    });
                }
            }
            let result = match &sig.ret {
                Some(ty) => self.ty(ty)?,
                None => Type::Unit,
            };
            self.program.funcs.push(Func {
                name: sig.name.clone(),
                params,
                result,
                line: sig.line,
            });
        }
        Ok(())
    }

    fn fields(&self, fields: &[ast::Field]) -> R<Vec<Field>> {
        let mut out = Vec::new();
        for field in fields {
            out.push(Field {
                name: field.name.clone(),
                ty: self.ty(&field.ty)?,
                line: field.line,
            });
        }
        Ok(out)
    }

    fn ty(&self, ty: &ast::Type) -> R<Type> {
        match &ty.kind {
            ast::TypeKind::Path(path) => self.path_type(path),
            ast::TypeKind::Unit => Ok(Type::Unit),
            ast::TypeKind::Tuple(items) => {
                let mut out = Vec::new();
                for item in items {
                    out.push(self.ty(item)?);
                }
                Ok(Type::Tuple(out))
            }
            ast::TypeKind::Any(_) => outside(ty.line, "any"),
            ast::TypeKind::Ref(..) => outside(ty.line, "ref"),
            ast::TypeKind::Ptr(..) => outside(ty.line, "ptr"),
            ast::TypeKind::Func(_) => outside(ty.line, "типы функций"),
        }
    }

    fn path_type(&self, path: &ast::PathType) -> R<Type> {
        if path.names.len() != 1 {
            return fail(path.line, format!("нет типа {}", path_text(path)));
        }
        let name = path.names[0].as_str();
        if name == "Array" {
            return self.array_type(path);
        }
        let found = if let Some(scalar) = Scalar::from_name(name) {
            Type::Scalar(scalar)
        } else if name == "Never" {
            Type::Never
        } else if let Some((Item::Type(id), _)) = self.names.get(name) {
            Type::Named(*id)
        } else if STANDARD_NAMES.contains(&name) {
            return outside(path.line, name);
        } else {
            return fail(path.line, format!("нет типа {}", name));
        };
        if !path.args.is_empty() {
            if let Type::Named(_) = found {
                return outside(path.line, "обобщения");
            }
            return fail(path.line, format!("тип {} пишется без параметров", name));
        }
        Ok(found)
    }

    /// `Array[T, N]`: длина — число от 0 до наибольшего `i64` (12.19, 12.37).
    fn array_type(&self, path: &ast::PathType) -> R<Type> {
        match path.args.as_slice() {
            [ast::GenericArg::Type(item), ast::GenericArg::Int(n)] => {
                let item = self.ty(item)?;
                match parse_int(n) {
                    Some(len) if len <= i64::MAX as u128 => {
                        Ok(Type::Array(Box::new(item), len as u64))
                    }
                    _ => fail(
                        path.line,
                        format!("длина массива {} больше наибольшего i64 (12.19)", n),
                    ),
                }
            }
            _ => fail(
                path.line,
                "массив пишется Array[T, N]: тип элемента и длина-число".to_string(),
            ),
        }
    }

    // ───────── impl Copy и простота ─────────

    fn copy_impls(&mut self) -> R<()> {
        for k in 0..self.impls.len() {
            let (target, line) = self.impls[k];
            let text = path_text(target);
            if !target.args.is_empty() {
                return outside(line, "обобщения");
            }
            if target.names.len() == 1 && builtin_type(&target.names[0]) {
                return fail(
                    line,
                    format!(
                        "impl Copy for {}: Copy объявляют только у записей и перечислений пакета",
                        text
                    ),
                );
            }
            let id = match self.names.get(text.as_str()) {
                Some((Item::Type(id), _)) if target.names.len() == 1 => *id,
                _ => return fail(line, format!("нет типа {}", text)),
            };
            if self.copy_lines.contains_key(&id) {
                return fail(line, format!("impl Copy for {} повторён", text));
            }
            self.copy_lines.insert(id, line);
            self.program.types[id].copy = true;
        }
        Ok(())
    }

    /// Тип не содержит сам себя ни прямо, ни через кортежи, массивы, записи и варианты:
    /// косвенности и кучи в шаге 2 нет (9.9). Обход — без рекурсии, по явному стеку: цепочка
    /// объявлений может быть сколь угодно длинной.
    fn self_containing(&self) -> R<()> {
        let types = &self.program.types;
        let mut edges: Vec<Vec<TypeId>> = Vec::new();
        for decl in types {
            let mut parts = Vec::new();
            match &decl.kind {
                TypeKind::Struct(fields) => {
                    for field in fields {
                        named_parts(&field.ty, &mut parts);
                    }
                }
                TypeKind::Enum(variants) => {
                    for variant in variants {
                        match &variant.payload {
                            Payload::None => {}
                            Payload::One(ty) => named_parts(ty, &mut parts),
                            Payload::Fields(fields) => {
                                for field in fields {
                                    named_parts(&field.ty, &mut parts);
                                }
                            }
                        }
                    }
                }
            }
            edges.push(parts);
        }
        // 0 — тип ещё не обойдён, 1 — он на текущем пути, 2 — обойдён целиком
        let mut state = vec![0u8; types.len()];
        for start in 0..types.len() {
            if state[start] != 0 {
                continue;
            }
            state[start] = 1;
            let mut path: Vec<(TypeId, usize)> = vec![(start, 0)];
            while let Some(top) = path.last_mut() {
                let (v, k) = *top;
                if k == edges[v].len() {
                    state[v] = 2;
                    path.pop();
                    continue;
                }
                top.1 += 1;
                let w = edges[v][k];
                if state[w] == 1 {
                    let from = path.iter().position(|(x, _)| *x == w).unwrap_or(0);
                    let mut chain: Vec<&str> = path[from..]
                        .iter()
                        .map(|(x, _)| types[*x].name.as_str())
                        .collect();
                    chain.push(types[w].name.as_str());
                    return fail(
                        types[w].line,
                        format!(
                            "тип {} содержит сам себя без косвенности: {} (9.9)",
                            types[w].name,
                            chain.join(" → ")
                        ),
                    );
                }
                if state[w] == 0 {
                    state[w] = 1;
                    path.push((w, 0));
                }
            }
        }
        Ok(())
    }

    /// У типа с `impl Copy` все поля и все значения вариантов простые (12.27, 10.2).
    fn copy_parts(&self) -> R<()> {
        let p = &self.program;
        for (id, decl) in p.types.iter().enumerate() {
            let Some(&line) = self.copy_lines.get(&id) else {
                continue;
            };
            match &decl.kind {
                TypeKind::Struct(fields) => {
                    for field in fields {
                        if !p.is_simple(&field.ty) {
                            return fail(
                                line,
                                format!(
                                    "impl Copy for {}: поле {} типа {} не простое",
                                    decl.name,
                                    field.name,
                                    p.type_text(&field.ty)
                                ),
                            );
                        }
                    }
                }
                TypeKind::Enum(variants) => {
                    for variant in variants {
                        match &variant.payload {
                            Payload::None => {}
                            Payload::One(ty) => {
                                if !p.is_simple(ty) {
                                    return fail(
                                        line,
                                        format!(
                                            "impl Copy for {}: значение варианта {} типа {} не простое",
                                            decl.name,
                                            variant.name,
                                            p.type_text(ty)
                                        ),
                                    );
                                }
                            }
                            Payload::Fields(fields) => {
                                for field in fields {
                                    if !p.is_simple(&field.ty) {
                                        return fail(
                                            line,
                                            format!(
                                                "impl Copy for {}: поле {} варианта {} типа {} не простое",
                                                decl.name,
                                                field.name,
                                                variant.name,
                                                p.type_text(&field.ty)
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Записи и перечисления, которые тип содержит без косвенности: сам тип, части кортежа,
/// элемент массива любой длины.
fn named_parts(ty: &Type, out: &mut Vec<TypeId>) {
    match ty {
        Type::Named(id) => out.push(*id),
        Type::Tuple(items) => {
            for item in items {
                named_parts(item, out);
            }
        }
        Type::Array(item, _) => named_parts(item, out),
        Type::Scalar(_) | Type::Unit | Type::Never => {}
    }
}

fn path_text(path: &ast::PathType) -> String {
    path.names.join(".")
}

/// Значение целого литерала: десятичного, `0x`, `0b` или `0o`, с разделителями `_` (12.55).
/// Запись проверил лексер; `None` — значение больше `u128`.
pub(crate) fn parse_int(text: &str) -> Option<u128> {
    let digits: String = text.chars().filter(|c| *c != '_').collect();
    let (radix, body) = if let Some(rest) = digits.strip_prefix("0x") {
        (16, rest)
    } else if let Some(rest) = digits.strip_prefix("0b") {
        (2, rest)
    } else if let Some(rest) = digits.strip_prefix("0o") {
        (8, rest)
    } else {
        (10, digits.as_str())
    };
    u128::from_str_radix(body, radix).ok()
}

/// Значение строки `"…"` без подстановок: экранирование снято (13.16). Запись экранирования
/// проверил лексер; здесь проверяется, что `\u{…}` — скалярное значение Unicode.
pub(crate) fn decode_string(raw: &str) -> Result<String, String> {
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(c @ ('\\' | '"' | '\'' | '$')) => out.push(c),
            Some('u') => {
                let mut digits = String::new();
                chars.next(); // «{»
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    digits.push(d);
                }
                match u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(ch) => out.push(ch),
                    None => {
                        return Err(format!("\\u{{{}}} — не скалярное значение Unicode", digits))
                    }
                }
            }
            _ => return Err("неизвестное экранирование в строке".to_string()),
        }
    }
    Ok(out)
}
