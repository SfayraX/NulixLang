//! Проверка явного ядра в исходнике: дерево разбора превращается в проверенное дерево `core`.
//!
//! Правила — `spec/core-check.md`. Проверка останавливается на первой ошибке и называет её
//! строку. Она не зависит ни от проверщика IR, ни от интерпретатора.
//!
//! Сейчас проверяются шаги 1–3 раздела 9 `spec/core-check.md`. Шаг 1 — файл, объявления и
//! сигнатуры: заголовок `@file`, конструкции вне шага 2, имена объявлений, типы полей, вариантов,
//! параметров и результатов, `impl Copy for T {}`, типы, которые содержат сами себя, метки `end`
//! у объявлений и имена тестов. Шаг 2 — выражения и инструкции в телах: локальные имена без
//! затенения, типы и литералы, операции, вызовы и режимы аргументов, места для записи, `if`,
//! циклы и их метки, `return`, `break`, `continue`, `assert`, недостижимый код и `Never`. Шаг 3
//! — `match`: образцы, имена из них, альтернативы, условия ветвей, режим `mut`, полнота и
//! недостижимые ветви; полноту и недостижимое считает `patterns.rs`. Владение — шаг 4 — пока
//! не проверяется.
//!
//! Порядок: сначала все объявления по порядку файла — их форма и имена; затем типы полей,
//! вариантов, параметров и результатов; затем `impl Copy`; затем типы, которые содержат сами
//! себя; затем простота частей у типов с `impl Copy`; затем тела функций и тестов по порядку
//! файла. Внутри тела условия `if` проверяются раньше его ветвей, а операнд и ветвь со своим
//! типом — раньше остальных (раздел 4, «Операции»). В `match` — значение, затем образцы и
//! условия ветвей по порядку, затем недостижимые ветви и полнота, затем тела ветвей, как у
//! `if`. Поэтому первой может оказаться ошибка правее или ниже по тексту. Рекурсия идёт по
//! вложенности типов, выражений, блоков и образцов, а её ограничивает предел вложенности
//! разборщика; вызывать проверку, как и разбор, нужно через `stack::run`.

use crate::ast;
use crate::core::{
    Arg, BinOp, Block, Body, Case, Expr, ExprKind, Field, Func, FuncId, Local, LocalId, LocalKind,
    LoopId, Match, Param, Pattern, PatternKind, Payload, Place, Program, Step, Stmt, StmtKind,
    Test, TuplePattern, Type, TypeDecl, TypeId, TypeKind, UnOp, Variant, VariantValue,
};
use crate::ir::{Mode, Scalar};
use crate::patterns;
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
/// Из интерфейсов операторов (12.90) спецификация поимённо называет только `Add` (12.25).
const STANDARD_NAMES: [&str; 24] = [
    "Option", "Result", "Some", "None", "Ok", "Err", "print", "Drop", "Copy", "From", "Error",
    "Display", "Debug", "Eq", "Ord", "Hash", "Iterator", "Add", "List", "Str", "Bytes", "TextOut",
    "size_of", "align_of",
];

/// Типы среди стандартных имён: в шаге 2 их нет (раздел 3). Остальные стандартные имена —
/// функции, варианты и интерфейсы — типами не являются.
const STANDARD_TYPES: [&str; 6] = ["Option", "Result", "List", "Str", "Bytes", "TextOut"];

/// Интерфейсы среди стандартных имён (12.90): значением они не бывают.
const STANDARD_INTERFACES: [&str; 11] = [
    "Drop", "Copy", "From", "Error", "Display", "Debug", "Eq", "Ord", "Hash", "Iterator", "Add",
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
    let order = c.self_containing()?;
    c.inhabited(&order);
    c.copy_parts()?;
    c.bodies()?;
    Ok(c.program)
}

/// Что объявлено под именем пакета.
#[derive(Debug, Clone, Copy)]
enum Item {
    Type(TypeId),
    Func(FuncId),
}

/// Чьё тело: функции или теста, по номеру.
#[derive(Debug, Clone, Copy)]
enum BodyOf {
    Func(FuncId),
    Test(usize),
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
    /// Функции из дерева разбора, по номерам в `program.funcs`.
    funcs: Vec<&'a ast::FuncDecl>,
    /// Тесты из дерева разбора, по номерам в `program.tests`.
    test_decls: Vec<&'a ast::TestDecl>,
    /// Тела функций и тестов в порядке файла.
    body_order: Vec<BodyOf>,
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
            ast::DeclKind::Struct(s) => self.struct_decl(s),
            ast::DeclKind::Enum(e) => self.enum_decl(e),
            ast::DeclKind::Interface(_) => outside(decl.line, "interface"),
            ast::DeclKind::Impl(i) => self.impl_decl(i),
            ast::DeclKind::TypeAlias(_) => outside(decl.line, "type"),
            ast::DeclKind::Test(t) => self.test_decl(t),
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
        let id = self.funcs.len();
        self.declare(&sig.name, sig.line, Item::Func(id))?;
        self.funcs.push(func);
        self.body_order.push(BodyOf::Func(id));
        Ok(())
    }

    fn struct_decl(&mut self, s: &'a ast::StructDecl) -> R<()> {
        let line = s.line;
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

    fn enum_decl(&mut self, e: &'a ast::EnumDecl) -> R<()> {
        let line = e.line;
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
                            field.ty.line,
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
    fn impl_decl(&mut self, i: &'a ast::ImplDecl) -> R<()> {
        let line = i.line;
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

    fn test_decl(&mut self, t: &'a ast::TestDecl) -> R<()> {
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
                format!(
                    "тест «{}» уже объявлен в строке {}",
                    value.escape_debug(),
                    first
                ),
            );
        }
        self.tests.insert(value.clone(), name.line);
        self.program.tests.push(Test {
            name: value,
            line: name.line,
            body: Body::default(),
        });
        self.body_order.push(BodyOf::Test(self.test_decls.len()));
        self.test_decls.push(t);
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
            // считается в `inhabited`, когда известны все части
            inhabited: false,
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
                                        line: field.ty.line,
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
            let sig = &self.funcs[k].sig;
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
                body: Body::default(),
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
        } else if STANDARD_TYPES.contains(&name) {
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
            if target.names.len() == 1
                && (builtin_type(&target.names[0]) || STANDARD_TYPES.contains(&text.as_str()))
            {
                return fail(
                    line,
                    format!(
                        "impl Copy for {}: Copy объявляют только у записей и перечислений пакета",
                        text
                    ),
                );
            }
            if !target.args.is_empty() {
                return outside(line, "обобщения");
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
    /// объявлений может быть сколь угодно длинной. Отдаёт типы в таком порядке, что каждый стоит
    /// после всех записей и перечислений, из которых он состоит.
    fn self_containing(&self) -> R<Vec<TypeId>> {
        let types = &self.program.types;
        let mut order = Vec::new();
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
                    order.push(v);
                    path.pop();
                    continue;
                }
                top.1 += 1;
                let w = edges[v][k];
                if state[w] == 1 {
                    let from = path.iter().position(|(x, _)| *x == w).unwrap_or(0);
                    let cycle = &path[from..];
                    // длинную цепочку сообщение показывает началом: первые 9 имён, «…» и конец
                    let shown = if cycle.len() > 9 { &cycle[..9] } else { cycle };
                    let mut chain: Vec<&str> =
                        shown.iter().map(|(x, _)| types[*x].name.as_str()).collect();
                    if cycle.len() > shown.len() {
                        chain.push("…");
                    }
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
        Ok(order)
    }

    /// Есть ли у записей и перечислений значения (решение 28). Типы идут так, что части каждого
    /// уже посчитаны, поэтому хватает одного прохода.
    fn inhabited(&mut self, order: &[TypeId]) {
        for &id in order {
            let p = &self.program;
            let inhabited = match &p.types[id].kind {
                TypeKind::Struct(fields) => fields.iter().all(|f| p.has_values(&f.ty)),
                TypeKind::Enum(variants) => {
                    variants.iter().any(|v| p.payload_has_values(&v.payload))
                }
            };
            self.program.types[id].inhabited = inhabited;
        }
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

// ───────── тела функций и тестов ─────────

const BOOL: Type = Type::Scalar(Scalar::Bool);
const CHAR: Type = Type::Scalar(Scalar::Char);
const I64: Type = Type::Scalar(Scalar::I64);

/// Символ из литерала: экранирование проверено, в символе ровно один знак.
fn char_value(raw: &str, line: u32) -> R<char> {
    let text = match decode_string(raw) {
        Ok(text) => text,
        Err(message) => return fail(line, message),
    };
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c),
        _ => fail(line, "в символе ровно один знак".to_string()),
    }
}

fn is_int(ty: &Type) -> bool {
    matches!(ty, Type::Scalar(s) if s.int_range().is_some())
}

fn is_float(ty: &Type) -> bool {
    matches!(ty, Type::Scalar(Scalar::F32 | Scalar::F64))
}

fn is_number(ty: &Type) -> bool {
    is_int(ty) || is_float(ty)
}

/// Последнее выражение блока, если блок кончается выражением.
fn tail(block: &ast::Block) -> Option<&ast::Expr> {
    match block.stmts.last() {
        Some(ast::Stmt {
            kind: ast::StmtKind::Expr(e),
            ..
        }) => Some(e),
        _ => None,
    }
}

/// Выражение без своего типа: его тип задаёт место или другой операнд (решения 2 и 24). Это
/// числовой литерал; скобки, минус и `~` вокруг такого выражения; арифметика и `&`, `|`, `^`,
/// где все операнды такие; сдвиг, где такой левый операнд; `if` с `else` и `match`, где значения
/// всех ветвей такие; кортеж и массив, где каждый элемент такой или литерал.
fn literal_like(e: &ast::Expr) -> bool {
    match &e.kind {
        ast::ExprKind::Int(_) | ast::ExprKind::Float(_) => true,
        ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => items.iter().all(literal_part),
        ast::ExprKind::Paren(inner) => literal_like(inner),
        ast::ExprKind::Unary(ast::UnOp::Neg | ast::UnOp::BitNot, inner) => literal_like(inner),
        ast::ExprKind::Binary(first, rest) => match rest.first().map(|(op, _)| *op) {
            Some(
                ast::BinOp::Add
                | ast::BinOp::Sub
                | ast::BinOp::Mul
                | ast::BinOp::Div
                | ast::BinOp::Rem
                | ast::BinOp::BitAnd
                | ast::BinOp::BitOr
                | ast::BinOp::BitXor,
            ) => literal_like(first) && rest.iter().all(|(_, o)| literal_like(o)),
            Some(ast::BinOp::Shl | ast::BinOp::Shr) => literal_like(first),
            _ => false,
        },
        ast::ExprKind::If(ifx) => match &ifx.otherwise {
            None => false,
            Some(otherwise) => {
                let tails: Vec<&ast::Expr> = ifx
                    .branches
                    .iter()
                    .map(|b| &b.then)
                    .chain(std::iter::once(otherwise))
                    .filter_map(tail)
                    .collect();
                !tails.is_empty() && tails.iter().all(|t| literal_like(t))
            }
        },
        ast::ExprKind::Match(m) => {
            let tails: Vec<&ast::Expr> = m.items.iter().map(item_block).filter_map(tail).collect();
            !tails.is_empty() && tails.iter().all(|t| literal_like(t))
        }
        _ => false,
    }
}

/// Порядок проверки ветвей `if` и `match`: сначала ветви, чьё значение со своим типом, затем
/// ветви со значением из литералов, затем ветви без значения (раздел 4, «Операции»).
fn branch_order(blocks: &[&ast::Block]) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    order.extend((0..blocks.len()).filter(|&k| tail(blocks[k]).is_some_and(|t| !literal_like(t))));
    order.extend((0..blocks.len()).filter(|&k| tail(blocks[k]).is_some_and(literal_like)));
    order.extend((0..blocks.len()).filter(|&k| tail(blocks[k]).is_none()));
    order
}

/// Тело ветви `match`: `case` или `else`.
fn item_block(item: &ast::MatchItem) -> &ast::Block {
    match item {
        ast::MatchItem::Case(c) => &c.body,
        ast::MatchItem::Else(b, ..) => b,
    }
}

/// Часть кортежа или массива из одних литералов: любой литерал (раздел 3) или выражение из
/// одних литералов.
fn literal_part(e: &ast::Expr) -> bool {
    matches!(
        e.kind,
        ast::ExprKind::Bool(_) | ast::ExprKind::Char(_) | ast::ExprKind::Unit
    ) || literal_like(e)
}

fn bin_op(op: ast::BinOp) -> Option<BinOp> {
    Some(match op {
        ast::BinOp::Add => BinOp::Add,
        ast::BinOp::Sub => BinOp::Sub,
        ast::BinOp::Mul => BinOp::Mul,
        ast::BinOp::Div => BinOp::Div,
        ast::BinOp::Rem => BinOp::Rem,
        ast::BinOp::BitAnd => BinOp::BitAnd,
        ast::BinOp::BitOr => BinOp::BitOr,
        ast::BinOp::BitXor => BinOp::BitXor,
        ast::BinOp::Shl => BinOp::Shl,
        ast::BinOp::Shr => BinOp::Shr,
        ast::BinOp::Eq => BinOp::Eq,
        ast::BinOp::Ne => BinOp::Ne,
        ast::BinOp::Lt => BinOp::Lt,
        ast::BinOp::Le => BinOp::Le,
        ast::BinOp::Gt => BinOp::Gt,
        ast::BinOp::Ge => BinOp::Ge,
        ast::BinOp::And | ast::BinOp::Or | ast::BinOp::RangeExcl | ast::BinOp::RangeIncl => {
            return None
        }
    })
}

/// Таблицы имён полей и вариантов: поиск по имени не перебирает объявление.
struct Tables<'p> {
    fields: Vec<HashMap<&'p str, usize>>,
    variants: Vec<HashMap<&'p str, usize>>,
    variant_fields: Vec<Vec<HashMap<&'p str, usize>>>,
    /// Первое перечисление с вариантом такого имени — для подсказки «пишется с именем типа».
    variant_owner: HashMap<&'p str, TypeId>,
}

impl<'p> Tables<'p> {
    fn new(p: &'p Program) -> Tables<'p> {
        let mut t = Tables {
            fields: Vec::new(),
            variants: Vec::new(),
            variant_fields: Vec::new(),
            variant_owner: HashMap::new(),
        };
        for (id, decl) in p.types.iter().enumerate() {
            let mut fields = HashMap::new();
            let mut variants = HashMap::new();
            let mut payloads = Vec::new();
            match &decl.kind {
                TypeKind::Struct(fs) => {
                    for (k, f) in fs.iter().enumerate() {
                        fields.insert(f.name.as_str(), k);
                    }
                }
                TypeKind::Enum(vs) => {
                    for (k, v) in vs.iter().enumerate() {
                        variants.insert(v.name.as_str(), k);
                        t.variant_owner.entry(v.name.as_str()).or_insert(id);
                        let mut names = HashMap::new();
                        if let Payload::Fields(fs) = &v.payload {
                            for (j, f) in fs.iter().enumerate() {
                                names.insert(f.name.as_str(), j);
                            }
                        }
                        payloads.push(names);
                    }
                }
            }
            t.fields.push(fields);
            t.variants.push(variants);
            t.variant_fields.push(payloads);
        }
        t
    }
}

/// Цикл, внутри которого идёт проверка.
struct LoopFrame<'a> {
    id: LoopId,
    label: Option<&'a str>,
    line: u32,
    has_break: bool,
}

impl<'a> Checker<'a> {
    /// Тела функций и тестов по порядку файла (`spec/core-check.md`, разделы 2 и 4).
    fn bodies(&mut self) -> R<()> {
        let mut funcs = vec![Body::default(); self.funcs.len()];
        let mut tests = vec![Body::default(); self.test_decls.len()];
        {
            let tables = Tables::new(&self.program);
            for &of in &self.body_order {
                match of {
                    BodyOf::Func(id) => {
                        let decl = self.funcs[id];
                        let f = &self.program.funcs[id];
                        let mut b = BodyChecker::new(self, &tables, f.result.clone());
                        for (param, source) in f.params.iter().zip(&decl.sig.params) {
                            if let ast::ParamKind::Named(name, _) = &source.kind {
                                b.declare(
                                    name,
                                    param.ty.clone(),
                                    LocalKind::Param(param.mode),
                                    param.line,
                                )?;
                            }
                        }
                        let block = b.block(&decl.body, Some(&f.result))?;
                        funcs[id] = Body {
                            locals: b.locals,
                            block,
                        };
                    }
                    BodyOf::Test(k) => {
                        let mut b = BodyChecker::new(self, &tables, Type::Unit);
                        let block = b.block(&self.test_decls[k].body, Some(&Type::Unit))?;
                        tests[k] = Body {
                            locals: b.locals,
                            block,
                        };
                    }
                }
            }
        }
        for (f, body) in self.program.funcs.iter_mut().zip(funcs) {
            f.body = body;
        }
        for (t, body) in self.program.tests.iter_mut().zip(tests) {
            t.body = body;
        }
        Ok(())
    }
}

/// Ветвь `match` после образцов и условия; тело проверяется потом.
enum Head<'a> {
    Case {
        patterns: Vec<Pattern>,
        guard: Option<Expr>,
        /// Имена из образцов с номерами — чтобы снова открыть их для тела.
        names: Vec<(&'a str, LocalId)>,
        body: &'a ast::Block,
        line: u32,
    },
    /// Тело `else` и строка слова `else`.
    Else(&'a ast::Block, u32),
}

/// Что недостижимо: ветвь целиком, её альтернатива с номером от 1 или `else`.
#[derive(Clone, Copy)]
enum Subject {
    Branch,
    Alternative(usize),
    Else,
}

/// Как связываются имена образца: первая альтернатива ветви объявляет их в режиме `match`,
/// остальные связывают те же имена тех же типов (12.43).
enum Binds<'b, 'a> {
    Declare(Mode),
    Same {
        first: &'b [(&'a str, LocalId)],
        seen: Vec<bool>,
        alternative: usize,
    },
}

/// Проверка одного тела.
struct BodyChecker<'c, 'a> {
    c: &'c Checker<'a>,
    p: &'c Program,
    t: &'c Tables<'c>,
    /// Результат функции; у теста — `()`.
    result: Type,
    locals: Vec<Local>,
    /// Видимые локальные имена. Затенения нет (12.93), поэтому у видимого имени одно значение.
    visible: HashMap<&'a str, LocalId>,
    /// Имена, объявленные в каждом из открытых блоков: при выходе из блока они перестают быть
    /// видны.
    scopes: Vec<Vec<&'a str>>,
    loops: Vec<LoopFrame<'a>>,
    loop_count: usize,
    /// Идёт условие `while`: циклы с номером в `loops` ниже этого — снаружи условия, из него
    /// `break` и `continue` к ним не относятся.
    cond_floor: Option<usize>,
    /// Следующий блок — ветвь `if` без `else`, чьё значение ждут: подсказка к решению 4.
    if_value: bool,
}

impl<'c, 'a> BodyChecker<'c, 'a> {
    fn new(c: &'c Checker<'a>, t: &'c Tables<'c>, result: Type) -> BodyChecker<'c, 'a> {
        BodyChecker {
            c,
            p: &c.program,
            t,
            result,
            locals: Vec::new(),
            visible: HashMap::new(),
            scopes: vec![Vec::new()],
            loops: Vec::new(),
            loop_count: 0,
            cond_floor: None,
            if_value: false,
        }
    }

    fn text(&self, ty: &Type) -> String {
        self.p.type_text(ty)
    }

    fn mismatch<T>(&self, line: u32, want: &Type, found: &Type) -> R<T> {
        fail(
            line,
            format!(
                "тип не совпадает: ожидался {}, здесь {}",
                self.text(want),
                self.text(found)
            ),
        )
    }

    /// Выражение подходит к ожидаемому типу; выражение, которое уходит, подходит к любому (9.36).
    fn expect(&self, e: Expr, want: Option<&Type>) -> R<Expr> {
        if let Some(want) = want {
            if e.ty != *want && !self.leaves(&e) {
                return self.mismatch(e.line, want, &e.ty);
            }
        }
        Ok(e)
    }

    /// Выражение уходит и значения не даёт: его тип — `Never`, или это вызов функции, чей
    /// результат — перечисление без вариантов (9.36, решение 10в).
    fn leaves(&self, e: &Expr) -> bool {
        e.ty == Type::Never || (matches!(e.kind, ExprKind::Call(..)) && self.p.is_never(&e.ty))
    }

    // ───────── имена ─────────

    /// Локальное имя: оно не совпадает с другим видимым локальным именем (12.93). Имя пакета,
    /// встроенного типа или стандартное имя оно скрыть может (раздел 4, решение 26).
    fn declare(&mut self, name: &'a str, ty: Type, kind: LocalKind, line: u32) -> R<LocalId> {
        if let Some(&other) = self.visible.get(name) {
            return fail(
                line,
                format!(
                    "имя {} уже объявлено в строке {}: затенения нет (12.93)",
                    name, self.locals[other].line
                ),
            );
        }
        let id = self.locals.len();
        self.locals.push(Local {
            name: name.to_string(),
            ty,
            kind,
            line,
        });
        self.visible.insert(name, id);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(name);
        }
        Ok(id)
    }

    /// Закрывает последнюю открытую область: её имена перестают быть видны. Отдаёт их с
    /// номерами.
    fn close_scope(&mut self) -> Vec<(&'a str, LocalId)> {
        let mut out = Vec::new();
        if let Some(names) = self.scopes.pop() {
            for name in names {
                if let Some(id) = self.visible.remove(name) {
                    out.push((name, id));
                }
            }
        }
        out
    }

    /// Снова открывает имена ветви `match` для её тела: на затенение их уже проверили, когда
    /// объявляли.
    fn reopen_scope(&mut self, names: &[(&'a str, LocalId)]) {
        self.scopes
            .push(names.iter().map(|&(name, _)| name).collect());
        for &(name, id) in names {
            self.visible.insert(name, id);
        }
    }

    /// В место можно писать: это `var`, параметр `mut` или `take` и имя из `match mut` (12.12,
    /// 12.58, 12.60, решение 16).
    fn writable(&self, local: LocalId, line: u32) -> R<()> {
        let l = &self.locals[local];
        match l.kind {
            LocalKind::Var
            | LocalKind::Param(Mode::Mut)
            | LocalKind::Param(Mode::Take)
            | LocalKind::Bound(Mode::Mut) => Ok(()),
            LocalKind::Bound(_) => fail(
                line,
                format!(
                    "{} — имя из образца: оно не меняется, кроме имён из match mut (решение 16)",
                    l.name
                ),
            ),
            LocalKind::Let => fail(
                line,
                format!(
                    "{} объявлено через let: его значение не меняется (12.12)",
                    l.name
                ),
            ),
            LocalKind::Param(Mode::Read) => fail(
                line,
                format!("{} — параметр для чтения: его не меняют (12.58)", l.name),
            ),
        }
    }

    // ───────── блоки и инструкции ─────────

    /// Блок. Если ожидается `()`, последнее выражение стоит инструкцией (решение 4); иначе оно
    /// — значение блока. После инструкции, которая всегда уходит, ничего нет (решение 7). Блок,
    /// который всегда уходит, имеет тип `Never` (9.36).
    fn block(&mut self, b: &'a ast::Block, want: Option<&Type>) -> R<Block> {
        // подсказка — только для последнего выражения этого блока, не для вложенных
        let if_value = std::mem::take(&mut self.if_value);
        self.scopes.push(Vec::new());
        let unit = want == Some(&Type::Unit);
        let mut stmts = Vec::new();
        let mut value = None;
        let mut ty = Type::Unit;
        let mut leaves: Option<u32> = None;
        let n = b.stmts.len();
        for (k, st) in b.stmts.iter().enumerate() {
            if let Some(at) = leaves {
                return fail(
                    st.line,
                    format!(
                        "сюда выполнение не дойдёт: инструкция в строке {} всегда уходит (решение 7)",
                        at
                    ),
                );
            }
            let last = k + 1 == n;
            match &st.kind {
                ast::StmtKind::Expr(e) if last && !unit => {
                    let v = self.expr(e, want)?;
                    if self.leaves(&v) {
                        ty = Type::Never;
                        stmts.push(Stmt {
                            kind: StmtKind::Expr(v),
                            line: st.line,
                        });
                    } else if v.ty == Type::Unit {
                        stmts.push(Stmt {
                            kind: StmtKind::Expr(v),
                            line: st.line,
                        });
                    } else {
                        ty = v.ty.clone();
                        value = Some(Box::new(v));
                    }
                }
                _ => {
                    let (s, always_leaves) = self.stmt(st, last && if_value)?;
                    if always_leaves {
                        leaves = Some(st.line);
                        ty = Type::Never;
                    }
                    stmts.push(s);
                }
            }
        }
        self.close_scope();
        if let Some(want) = want {
            if ty != *want && ty != Type::Never {
                let message = match b.stmts.last() {
                    None => format!(
                        "блок пуст, а должен дать значение типа {}",
                        self.text(want)
                    ),
                    Some(_) => format!(
                        "блок должен дать значение типа {}, а его последняя инструкция значения не даёт",
                        self.text(want)
                    ),
                };
                let line = b.stmts.last().map_or(b.line, |st| st.line);
                return fail(line, message);
            }
        }
        Ok(Block {
            stmts,
            value,
            ty,
            line: b.line,
        })
    }

    /// Инструкция и признак того, что она всегда уходит. `if_value` — инструкция последняя в
    /// ветви `if` без `else`, чьё значение ждут.
    fn stmt(&mut self, st: &'a ast::Stmt, if_value: bool) -> R<(Stmt, bool)> {
        let line = st.line;
        let (kind, leaves) = match &st.kind {
            ast::StmtKind::Binding(b) => (self.binding(b)?, false),
            ast::StmtKind::Expr(e) => {
                // ветви if и match в инструкции — тоже инструкции (решение 4); скобки этого не
                // меняют
                let mut bare = e;
                while let ast::ExprKind::Paren(inner) = &bare.kind {
                    bare = inner;
                }
                let v = if matches!(bare.kind, ast::ExprKind::If(_) | ast::ExprKind::Match(_)) {
                    self.expr(bare, Some(&Type::Unit))?
                } else {
                    self.expr(bare, None)?
                };
                let leaves = self.leaves(&v);
                if !leaves && v.ty != Type::Unit && !matches!(v.kind, ExprKind::Call(..)) {
                    let message = if if_value {
                        format!(
                            "if без else даёт (), и значение типа {} здесь пропадёт (решение 4): допишите else",
                            self.text(&v.ty)
                        )
                    } else {
                        format!(
                            "значение типа {} не используется: выбросить можно только результат вызова функции (решение 4)",
                            self.text(&v.ty)
                        )
                    };
                    return fail(e.line, message);
                }
                (StmtKind::Expr(v), leaves)
            }
            ast::StmtKind::Assign(op, place, value) => {
                (self.assign(*op, place, value, line)?, false)
            }
            ast::StmtKind::For(_) => return outside(line, "for"),
            ast::StmtKind::While(w) => (self.while_loop(w, line)?, false),
            ast::StmtKind::Loop(l) => self.endless_loop(l, line)?,
            ast::StmtKind::Return(value) => (self.return_stmt(value.as_ref(), line)?, true),
            ast::StmtKind::Break(label) => (
                StmtKind::Break(self.loop_target(label.as_deref(), line, true)?),
                true,
            ),
            ast::StmtKind::Continue(label) => (
                StmtKind::Continue(self.loop_target(label.as_deref(), line, false)?),
                true,
            ),
            ast::StmtKind::Assert(cond, message) => {
                (self.assert_stmt(cond, message.as_ref())?, false)
            }
            ast::StmtKind::DiscardSelf => return outside(line, "методы и self"),
        };
        Ok((Stmt { kind, line }, leaves))
    }

    /// `let` и `var`: тип — написанный или правой части (решение 1).
    fn binding(&mut self, b: &'a ast::Binding) -> R<StmtKind> {
        let kind = match b.kind {
            ast::BindKind::Let => LocalKind::Let,
            ast::BindKind::Var => LocalKind::Var,
            ast::BindKind::Const => return outside(b.line, "const"),
        };
        let written = match &b.ty {
            Some(ty) => Some(self.c.ty(ty)?),
            None => None,
        };
        let value = self.inner(&b.value, written.as_ref())?;
        match &b.target {
            ast::BindTarget::Name(name) => {
                let ty = written.unwrap_or_else(|| value.ty.clone());
                let id = self.declare(name, ty, kind, b.line)?;
                Ok(StmtKind::Let(id, value))
            }
            ast::BindTarget::Tuple(pattern) => {
                let ty = written.unwrap_or_else(|| value.ty.clone());
                let pattern = self.tuple_pattern(pattern, &ty, kind)?;
                Ok(StmtKind::LetTuple(pattern, value))
            }
        }
    }

    /// Образец в `let`: имена, `_` и вложенные кортежи — он подходит всегда (решение 18).
    fn tuple_pattern(
        &mut self,
        p: &'a ast::Pattern,
        ty: &Type,
        kind: LocalKind,
    ) -> R<TuplePattern> {
        match &p.kind {
            ast::PatternKind::Wildcard => Ok(TuplePattern::Skip),
            ast::PatternKind::Path(names) if names.len() == 1 => {
                let name = names[0].as_str();
                if let Type::Named(id) = ty {
                    if self.t.variants[*id].contains_key(name) {
                        return fail(
                            p.line,
                            format!(
                                "в let образец подходит всегда, а {} — вариант {} (решение 18)",
                                name, self.p.types[*id].name
                            ),
                        );
                    }
                }
                Ok(TuplePattern::Bind(self.declare(
                    name,
                    ty.clone(),
                    kind,
                    p.line,
                )?))
            }
            ast::PatternKind::Tuple(items) => match ty {
                Type::Tuple(parts) if parts.len() == items.len() => {
                    let mut out = Vec::new();
                    for (item, part) in items.iter().zip(parts) {
                        out.push(self.tuple_pattern(item, part, kind)?);
                    }
                    Ok(TuplePattern::Tuple(out))
                }
                _ => fail(
                    p.line,
                    format!(
                        "образец из {} частей, а значение типа {}",
                        items.len(),
                        self.text(ty)
                    ),
                ),
            },
            _ => fail(
                p.line,
                "в let образец подходит всегда: только имена, _ и кортежи (решение 18)".to_string(),
            ),
        }
    }

    fn assign(
        &mut self,
        op: Option<ast::BinOp>,
        target: &'a ast::Expr,
        value: &'a ast::Expr,
        line: u32,
    ) -> R<StmtKind> {
        let (place, ty) = self.place(
            target,
            "присваивают месту: переменной, параметру, их полю или элементу",
        )?;
        match op {
            None => {
                let value = self.inner(value, Some(&ty))?;
                Ok(StmtKind::Assign(place, None, value))
            }
            Some(op) => {
                let Some(op) = bin_op(op) else {
                    return fail(line, "такого составного присваивания нет".to_string());
                };
                self.op_fits(op, &ty, line)?;
                let value = if matches!(op, BinOp::Shl | BinOp::Shr) {
                    let amount = self.inner(value, None)?;
                    self.shift_amount(&amount)?;
                    amount
                } else {
                    self.inner(value, Some(&ty))?
                };
                Ok(StmtKind::Assign(place, Some(op), value))
            }
        }
    }

    /// Место для записи или для `mut`: локальное имя, его поля и элементы; в него можно писать.
    /// `not_place` — сообщение, если выражение — не место.
    fn place(&mut self, e: &'a ast::Expr, not_place: &str) -> R<(Place, Type)> {
        let base_line = e.line;
        // имя, которое ничего не называет, — ошибка имени, а не места
        let base = match &e.kind {
            ast::ExprKind::Postfix(base, _) => base,
            _ => e,
        };
        if let ast::ExprKind::Name(name) = &base.kind {
            if !self.names_something(name) {
                return fail(base_line, format!("нет имени {}", name));
            }
        }
        let is_place = match &e.kind {
            ast::ExprKind::Name(name) => self.visible.contains_key(name.as_str()),
            ast::ExprKind::Postfix(base, ops) => {
                matches!(&base.kind, ast::ExprKind::Name(name) if self.visible.contains_key(name.as_str()))
                    && ops.iter().all(|op| !matches!(op, ast::PostfixOp::Call(_)))
            }
            _ => false,
        };
        if !is_place {
            return fail(base_line, not_place.to_string());
        }
        let v = self.expr(e, None)?;
        match v.kind {
            ExprKind::Place(place) => {
                self.writable(place.base, base_line)?;
                Ok((place, v.ty))
            }
            _ => fail(base_line, not_place.to_string()),
        }
    }

    /// `while`: условие — не тело цикла, поэтому `break` и `continue` в условии бывают только у
    /// циклов внутри него (решение 23).
    fn while_loop(&mut self, w: &'a ast::WhileStmt, line: u32) -> R<StmtKind> {
        let id = self.enter_loop(w.label.as_deref(), line)?;
        let outer = self.cond_floor.replace(self.loops.len());
        let cond = self.inner(&w.cond, Some(&BOOL))?;
        self.cond_floor = outer;
        let body = self.block(&w.body, Some(&Type::Unit))?;
        self.loops.pop();
        self.loop_end(w.end.as_ref(), "while", w.label.as_deref(), line)?;
        Ok(StmtKind::While(id, cond, body))
    }

    /// `loop`: без `break` своего цикла он всегда уходит (9.36).
    fn endless_loop(&mut self, l: &'a ast::LoopStmt, line: u32) -> R<(StmtKind, bool)> {
        let id = self.enter_loop(l.label.as_deref(), line)?;
        let body = self.block(&l.body, Some(&Type::Unit))?;
        let has_break = self.loops.pop().is_some_and(|f| f.has_break);
        self.loop_end(l.end.as_ref(), "loop", l.label.as_deref(), line)?;
        Ok((StmtKind::Loop(id, body), !has_break))
    }

    /// Вложенные циклы с одной меткой — ошибка, как в IR (решение 10ж).
    fn enter_loop(&mut self, label: Option<&'a str>, line: u32) -> R<LoopId> {
        if let Some(label) = label {
            if let Some(outer) = self.loops.iter().find(|f| f.label == Some(label)) {
                return fail(
                    line,
                    format!(
                        "метка {} уже у внешнего цикла в строке {} (решение 10ж)",
                        label, outer.line
                    ),
                );
            }
        }
        let id = self.loop_count;
        self.loop_count += 1;
        self.loops.push(LoopFrame {
            id,
            label,
            line,
            has_break: false,
        });
        Ok(id)
    }

    /// Метка `end` у цикла: у помеченного цикла — его метка, у остального — слово цикла (12.8,
    /// решения 12 и 22).
    fn loop_end(
        &self,
        end: Option<&ast::EndLoop>,
        word: &str,
        label: Option<&str>,
        line: u32,
    ) -> R<()> {
        let text = match end {
            None => return Ok(()),
            Some(ast::EndLoop::Keyword(k)) => k.as_str(),
            Some(ast::EndLoop::Label(l)) => l.as_str(),
        };
        match label {
            Some(label) if text != label => fail(
                line,
                format!("метка end {} не совпадает с меткой цикла {}", text, label),
            ),
            None if text != word => fail(
                line,
                format!("метка end {} не совпадает с началом цикла {}", text, word),
            ),
            _ => Ok(()),
        }
    }

    fn loop_target(&mut self, label: Option<&str>, line: u32, is_break: bool) -> R<LoopId> {
        let word = if is_break { "break" } else { "continue" };
        let k = match label {
            None => match self.loops.len().checked_sub(1) {
                Some(k) => k,
                None => return fail(line, format!("{} стоит вне цикла", word)),
            },
            Some(label) => match self.loops.iter().rposition(|f| f.label == Some(label)) {
                Some(k) => k,
                None => return fail(line, format!("{}: нет цикла с меткой {}", word, label)),
            },
        };
        if self.cond_floor.is_some_and(|floor| k < floor) {
            return fail(
                line,
                format!(
                    "{} стоит в условии while, а условие — не тело цикла (решение 23): перенесите проверку в тело",
                    word
                ),
            );
        }
        if is_break {
            self.loops[k].has_break = true;
        }
        Ok(self.loops[k].id)
    }

    fn return_stmt(&mut self, value: Option<&'a ast::Expr>, line: u32) -> R<StmtKind> {
        match value {
            Some(value) => {
                let result = self.result.clone();
                Ok(StmtKind::Return(Some(self.inner(value, Some(&result))?)))
            }
            None if self.result == Type::Unit => Ok(StmtKind::Return(None)),
            None => fail(
                line,
                format!(
                    "return без значения, а результат функции — {}",
                    self.text(&self.result)
                ),
            ),
        }
    }

    fn assert_stmt(&mut self, cond: &'a ast::Expr, message: Option<&'a ast::Expr>) -> R<StmtKind> {
        let cond = self.inner(cond, Some(&BOOL))?;
        let message = match message {
            None => None,
            Some(e) => match &e.kind {
                ast::ExprKind::Str(lit) if !lit.subs.is_empty() => {
                    return fail(
                        lit.line,
                        "сообщение assert — строка без подстановок (решение 10к)".to_string(),
                    );
                }
                ast::ExprKind::Str(lit) if lit.raw.starts_with("\"\"\"") => {
                    return outside(lit.line, "многострочная строка в сообщении assert");
                }
                ast::ExprKind::Str(lit) => match decode_string(&lit.raw) {
                    Ok(text) => Some(text),
                    Err(message) => return fail(lit.line, message),
                },
                _ => {
                    return fail(
                        e.line,
                        "сообщение assert — строка: assert условие, \"пояснение\" (12.48)"
                            .to_string(),
                    )
                }
            },
        };
        Ok(StmtKind::Assert(cond, message))
    }

    // ───────── выражения ─────────

    /// Выражение не в конце блока и не инструкцией: его значение используется, поэтому типа без
    /// значений здесь быть не может (решение 14).
    fn inner(&mut self, e: &'a ast::Expr, want: Option<&Type>) -> R<Expr> {
        let v = self.expr(e, want)?;
        self.valued(v)
    }

    /// Значение используется дальше, поэтому его тип — не тип без значений (решение 14).
    fn valued(&self, v: Expr) -> R<Expr> {
        if self.leaves(&v) {
            return fail(
                v.line,
                format!(
                    "выражение типа {} значения не даёт, и дальше выполнение не пойдёт (решения 7 и 14): его ставят отдельной инструкцией или последним в блоке",
                    self.text(&v.ty)
                ),
            );
        }
        Ok(v)
    }

    fn expr(&mut self, e: &'a ast::Expr, want: Option<&Type>) -> R<Expr> {
        let line = e.line;
        let v = match &e.kind {
            ast::ExprKind::Int(text) => self.int_literal(text, false, line, want)?,
            ast::ExprKind::Float(text) => self.float_literal(text, false, line, want)?,
            // минус прямо перед числом — часть литерала (решения 10а и 25)
            ast::ExprKind::Unary(ast::UnOp::Neg, inner) => match &inner.kind {
                ast::ExprKind::Int(text) => self.int_literal(text, true, line, want)?,
                ast::ExprKind::Float(text) => self.float_literal(text, true, line, want)?,
                _ => self.unary(ast::UnOp::Neg, inner, line, want)?,
            },
            ast::ExprKind::Bool(b) => Expr {
                kind: ExprKind::Bool(*b),
                ty: BOOL,
                line,
            },
            ast::ExprKind::Char(raw) => Expr {
                kind: ExprKind::Char(char_value(raw, line)?),
                ty: CHAR,
                line,
            },
            ast::ExprKind::Unit => Expr {
                kind: ExprKind::Unit,
                ty: Type::Unit,
                line,
            },
            ast::ExprKind::Str(_) => return outside(line, "строки"),
            ast::ExprKind::SelfValue => return outside(line, "методы и self"),
            ast::ExprKind::Name(_) | ast::ExprKind::Postfix(..) => self.postfix(e)?,
            ast::ExprKind::Paren(inner) => return self.expr(inner, want),
            ast::ExprKind::Tuple(items) => {
                let parts = match want {
                    Some(Type::Tuple(parts)) if parts.len() == items.len() => Some(parts.clone()),
                    _ => None,
                };
                let mut out = Vec::new();
                for (k, item) in items.iter().enumerate() {
                    let part = parts.as_ref().map(|p| &p[k]);
                    out.push(self.inner(item, part)?);
                }
                let ty = Type::Tuple(out.iter().map(|v| v.ty.clone()).collect());
                Expr {
                    kind: ExprKind::Tuple(out),
                    ty,
                    line,
                }
            }
            ast::ExprKind::List(items) => self.array(items, line, want)?,
            ast::ExprKind::Binary(first, rest) => self.binary(first, rest, line, want)?,
            ast::ExprKind::Unary(op, inner) => self.unary(*op, inner, line, want)?,
            ast::ExprKind::Is(..) => return outside(line, "is"),
            ast::ExprKind::Catch(_) => return outside(line, "catch"),
            ast::ExprKind::If(ifx) => self.if_expr(ifx, line, want)?,
            ast::ExprKind::Match(m) => self.match_expr(m, line, want)?,
            ast::ExprKind::Closure(_) => return outside(line, "замыкания"),
            ast::ExprKind::Trust(_) => return outside(line, "trust"),
            ast::ExprKind::Provide(_) => return outside(line, "provide"),
            ast::ExprKind::Target(_) => return outside(line, "target"),
            ast::ExprKind::LevelBlock(_) => {
                return outside(line, "блоки с пометкой уровня или строгости")
            }
        };
        self.expect(v, want)
    }

    /// Целый литерал: тип места, иначе `i64`; значение помещается в тип (12.56, решение 2).
    fn int_literal(&self, text: &str, negative: bool, line: u32, want: Option<&Type>) -> R<Expr> {
        let (value, scalar) = self.int_value(text, negative, line, want)?;
        Ok(Expr {
            kind: ExprKind::Int(value),
            ty: Type::Scalar(scalar),
            line,
        })
    }

    /// Значение и тип целого литерала: тип по месту, без места — `i64` (решение 2, 12.56).
    fn int_value(
        &self,
        text: &str,
        negative: bool,
        line: u32,
        want: Option<&Type>,
    ) -> R<(i128, Scalar)> {
        let shown = if negative {
            format!("-{}", text)
        } else {
            text.to_string()
        };
        let scalar = match want {
            Some(Type::Scalar(s)) if s.int_range().is_some() => *s,
            Some(ty) if is_float(ty) => {
                // в совете — десятичная запись: дробные пишутся только так (13.16)
                let decimal = match parse_int(text) {
                    Some(m) if negative => format!("-{}", m),
                    Some(m) => m.to_string(),
                    None => shown.clone(),
                };
                return fail(
                    line,
                    format!(
                        "целое число {} на месте дробного {}: пишут {}.0 (решение 2)",
                        shown,
                        self.text(ty),
                        decimal
                    ),
                );
            }
            Some(ty) => {
                return fail(
                    line,
                    format!(
                        "тип не совпадает: ожидался {}, здесь целое число",
                        self.text(ty)
                    ),
                )
            }
            None => Scalar::I64,
        };
        if negative && !scalar.is_signed() {
            return fail(
                line,
                format!("у беззнакового {} нет смены знака (9.18)", scalar.name()),
            );
        }
        let (lo, hi) = scalar.int_range().unwrap_or((0, 0));
        // число больше i128 не помещается ни в один тип: его заменяет i128::MAX
        let value = match parse_int(text).map(i128::try_from) {
            Some(Ok(m)) if negative => -m,
            Some(Ok(m)) => m,
            _ => i128::MAX,
        };
        if value < lo || value > hi {
            return fail(
                line,
                format!("число {} не помещается в {} (12.56)", shown, scalar.name()),
            );
        }
        Ok((value, scalar))
    }

    /// Дробный литерал: `f32` или `f64` по месту, без места — `f64`; остаётся конечным (12.56).
    /// Десятичная запись округляется прямо до своего типа.
    fn float_literal(&self, text: &str, negative: bool, line: u32, want: Option<&Type>) -> R<Expr> {
        let shown = if negative {
            format!("-{}", text)
        } else {
            text.to_string()
        };
        let scalar = match want {
            Some(Type::Scalar(s @ (Scalar::F32 | Scalar::F64))) => *s,
            Some(ty) if is_int(ty) => {
                return fail(
                    line,
                    format!(
                        "дробное число {} на месте целого {} (решение 2)",
                        shown,
                        self.text(ty)
                    ),
                )
            }
            Some(ty) => {
                return fail(
                    line,
                    format!(
                        "тип не совпадает: ожидался {}, здесь дробное число",
                        self.text(ty)
                    ),
                )
            }
            None => Scalar::F64,
        };
        let digits: String = text.chars().filter(|c| *c != '_').collect();
        let value = if scalar == Scalar::F32 {
            digits.parse::<f32>().ok().map(f64::from)
        } else {
            digits.parse::<f64>().ok()
        };
        match value {
            Some(v) if v.is_finite() => Ok(Expr {
                kind: ExprKind::Float(if negative { -v } else { v }),
                ty: Type::Scalar(scalar),
                line,
            }),
            _ => fail(
                line,
                format!("число {} не помещается в {} (12.56)", shown, scalar.name()),
            ),
        }
    }

    fn array(&mut self, items: &'a [ast::Expr], line: u32, want: Option<&Type>) -> R<Expr> {
        match want {
            Some(Type::Array(item, n)) => {
                if items.len() as u64 != *n {
                    return fail(
                        line,
                        format!(
                            "в массиве {} должно быть элементов: {}, а здесь {}",
                            self.text(&Type::Array(item.clone(), *n)),
                            n,
                            items.len()
                        ),
                    );
                }
                let mut out = Vec::new();
                for e in items {
                    out.push(self.inner(e, Some(item))?);
                }
                Ok(Expr {
                    kind: ExprKind::Array(out),
                    ty: Type::Array(item.clone(), *n),
                    line,
                })
            }
            Some(ty) => fail(
                line,
                format!(
                    "тип не совпадает: ожидался {}, здесь массив",
                    self.text(ty)
                ),
            ),
            None => fail(
                line,
                "у массива не указан тип: без типа [ … ] — список, а его в шаге 2 нет (12.37); укажите Array[T, N]"
                    .to_string(),
            ),
        }
    }

    fn unary(
        &mut self,
        op: ast::UnOp,
        inner: &'a ast::Expr,
        line: u32,
        want: Option<&Type>,
    ) -> R<Expr> {
        let (op, value) = match op {
            ast::UnOp::Try => return outside(line, "try"),
            ast::UnOp::Await => return outside(line, "await"),
            ast::UnOp::Not => (UnOp::Not, self.inner(inner, Some(&BOOL))?),
            ast::UnOp::Neg => {
                let want = want.filter(|t| is_number(t));
                let v = self.inner(inner, want)?;
                let signed = matches!(&v.ty, Type::Scalar(s) if s.is_signed()) || is_float(&v.ty);
                if !signed {
                    return fail(
                        line,
                        format!(
                            "смена знака есть только у знаковых целых и дробных (9.18), а здесь {}",
                            self.text(&v.ty)
                        ),
                    );
                }
                (UnOp::Neg, v)
            }
            ast::UnOp::BitNot => {
                let want = want.filter(|t| is_int(t));
                let v = self.inner(inner, want)?;
                if !is_int(&v.ty) {
                    return fail(
                        line,
                        format!(
                            "~ — только у целых (решение 10г), а здесь {}",
                            self.text(&v.ty)
                        ),
                    );
                }
                (UnOp::BitNot, v)
            }
        };
        let ty = value.ty.clone();
        Ok(Expr {
            kind: ExprKind::Unary(op, Box::new(value)),
            ty,
            line,
        })
    }

    /// Цепочка двухместных операций одного семейства (13.5). Семейства не смешивает разбор:
    /// `and` и `or` — каждое своей цепочкой, сравнение и сдвиг — по одному, арифметика — `+` и
    /// `-` или `*`, `/` и `%`, а `&`, `|` и `^` — каждый своей цепочкой. Поэтому у всех
    /// операций цепочки одно правило для операндов, и оно проверяется один раз.
    fn binary(
        &mut self,
        first: &'a ast::Expr,
        rest: &'a [(ast::BinOp, ast::Expr)],
        line: u32,
        want: Option<&Type>,
    ) -> R<Expr> {
        let Some(&(op0, _)) = rest.first() else {
            return self.expr(first, want);
        };
        match op0 {
            ast::BinOp::And | ast::BinOp::Or => {
                let mut parts = vec![self.inner(first, Some(&BOOL))?];
                for (_, e) in rest {
                    parts.push(self.inner(e, Some(&BOOL))?);
                }
                let kind = if op0 == ast::BinOp::And {
                    ExprKind::And(parts)
                } else {
                    ExprKind::Or(parts)
                };
                return Ok(Expr {
                    kind,
                    ty: BOOL,
                    line,
                });
            }
            ast::BinOp::RangeExcl | ast::BinOp::RangeIncl => {
                return outside(line, "диапазоны ..< и ..=")
            }
            _ => {}
        }
        let mut ops = Vec::new();
        for (op, _) in rest {
            match bin_op(*op) {
                Some(op) => ops.push(op),
                None => {
                    return fail(
                        line,
                        "в цепочке операции разных семейств (13.5)".to_string(),
                    )
                }
            }
        }
        let operands: Vec<&'a ast::Expr> = std::iter::once(first)
            .chain(rest.iter().map(|(_, e)| e))
            .collect();
        let (typed, ty) = match ops[..] {
            [op @ (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)] => {
                // сравнение: тип операндов — от операнда со своим типом, место не влияет
                (self.same_type(&operands, None, op, line)?.0, BOOL)
            }
            [BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, ..] => {
                return fail(
                    line,
                    "сравнения не выстраиваются в цепочку (13.5)".to_string(),
                )
            }
            [op @ (BinOp::Shl | BinOp::Shr), ..] => {
                // литерал слева берёт тип от места, иначе это i64; величина сдвига — целое
                // любого типа, литерал в ней — i64 (9.13)
                let want = want.filter(|t| op_ok(op, t));
                let left = if want.is_none() && literal_like(first) {
                    self.inner(first, Some(&I64))?
                } else {
                    self.inner(first, want)?
                };
                self.op_fits(op, &left.ty, line)?;
                let ty = left.ty.clone();
                let mut typed = vec![left];
                for (_, e) in rest {
                    let amount = self.inner(e, None)?;
                    self.shift_amount(&amount)?;
                    typed.push(amount);
                }
                (typed, ty)
            }
            [op, ..] => {
                // арифметика, `&`, `|`, `^`: операнды одного типа — от места, иначе от операнда
                // со своим типом (решение 2). Место, на котором операции нет, тип не даёт: тогда
                // ошибка — несовпадение типа результата с местом
                self.same_type(&operands, want.filter(|t| op_ok(op, t)), op, line)?
            }
            [] => return self.expr(first, want),
        };
        let mut typed = typed.into_iter();
        let Some(head) = typed.next() else {
            return fail(line, "нет операндов".to_string());
        };
        Ok(Expr {
            kind: ExprKind::Binary(Box::new(head), ops.into_iter().zip(typed).collect()),
            ty,
            line,
        })
    }

    /// Операнды одного типа: от места, если оно задано; иначе от первого операнда со своим
    /// типом; если такого нет — тип литерала без места (решение 2). Операнд со своим типом
    /// проверяется первым, остальные — по его типу. Отдаёт операнды по порядку и их тип.
    fn same_type(
        &mut self,
        operands: &[&'a ast::Expr],
        want: Option<&Type>,
        op: BinOp,
        line: u32,
    ) -> R<(Vec<Expr>, Type)> {
        let mut typed: Vec<Option<Expr>> = vec![None; operands.len()];
        let ty = match want {
            Some(ty) => ty.clone(),
            None => {
                // если операнда со своим типом нет, первым идёт первый операнд: его литералы
                // получают тип без места — `i64` или `f64` (12.56). Если в операции есть и
                // целые, и дробные, ошибка будет в любом случае, а от первого литерала зависит
                // только то, о каком литерале она скажет
                let k = operands.iter().position(|o| !literal_like(o)).unwrap_or(0);
                let v = self.inner(operands[k], None)?;
                let ty = v.ty.clone();
                typed[k] = Some(v);
                ty
            }
        };
        self.op_fits(op, &ty, line)?;
        let mut out = Vec::new();
        for (k, slot) in typed.into_iter().enumerate() {
            out.push(match slot {
                Some(v) => v,
                None => self.inner(operands[k], Some(&ty))?,
            });
        }
        Ok((out, ty))
    }

    /// Тип операндов подходит операции (раздел 4, «Операции»).
    fn op_fits(&self, op: BinOp, ty: &Type, line: u32) -> R<()> {
        if op_ok(op, ty) {
            return Ok(());
        }
        let need = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => "у целых и дробных",
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                "только у целых (решение 10г)"
            }
            BinOp::Eq | BinOp::Ne => "у скаляров (решение 10д)",
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                "у целых, дробных и символов (решение 10д)"
            }
        };
        fail(
            line,
            format!("{} — {}, а здесь {}", op_text(op), need, self.text(ty)),
        )
    }

    fn shift_amount(&self, amount: &Expr) -> R<()> {
        if is_int(&amount.ty) {
            Ok(())
        } else {
            fail(
                amount.line,
                format!(
                    "величина сдвига — целое (9.13), а здесь {}",
                    self.text(&amount.ty)
                ),
            )
        }
    }

    /// `if`: ветви одного типа — от места, иначе от ветви со своим типом (раздел 4). Ветвь со
    /// своим типом проверяется первой. Без `else` тип `if` — `()`.
    fn if_expr(&mut self, ifx: &'a ast::IfExpr, line: u32, want: Option<&Type>) -> R<Expr> {
        let mut conds = Vec::new();
        for branch in &ifx.branches {
            conds.push(self.inner(&branch.cond, Some(&BOOL))?);
        }
        let Some(otherwise) = &ifx.otherwise else {
            if let Some(want) = want {
                if *want != Type::Unit {
                    return fail(
                        line,
                        format!("if без else даёт (), а ожидался {}", self.text(want)),
                    );
                }
            }
            let mut branches = Vec::new();
            for (branch, cond) in ifx.branches.iter().zip(conds) {
                // значение такого if ждут, но у ветвей его нет: подсказка к решению 4
                self.if_value = want.is_none();
                branches.push((cond, self.block(&branch.then, Some(&Type::Unit))?));
            }
            return Ok(Expr {
                kind: ExprKind::If(branches, None),
                ty: Type::Unit,
                line,
            });
        };
        let blocks: Vec<&'a ast::Block> = ifx
            .branches
            .iter()
            .map(|b| &b.then)
            .chain(std::iter::once(otherwise))
            .collect();
        let mut ty: Option<Type> = want.cloned();
        let mut checked: Vec<Option<Block>> = vec![None; blocks.len()];
        for k in branch_order(&blocks) {
            let b = self.block(blocks[k], ty.as_ref())?;
            if ty.is_none() && b.ty != Type::Never {
                ty = Some(b.ty.clone());
            }
            checked[k] = Some(b);
        }
        let mut checked = checked.into_iter().map(|b| b.unwrap_or_default());
        let mut branches = Vec::new();
        for cond in conds {
            branches.push((cond, checked.next().unwrap_or_default()));
        }
        let otherwise = checked.next().unwrap_or_default();
        let all_leave =
            branches.iter().all(|(_, b)| b.ty == Type::Never) && otherwise.ty == Type::Never;
        let ty = if all_leave {
            Type::Never
        } else {
            ty.unwrap_or(Type::Unit)
        };
        Ok(Expr {
            kind: ExprKind::If(branches, Some(otherwise)),
            ty,
            line,
        })
    }

    // ───────── match ─────────

    /// `match` (12.41, раздел 6). Сначала значение, затем образцы и условия ветвей по порядку,
    /// затем недостижимые ветви и полнота, затем тела — как ветви `if`: ветвь со своим типом
    /// раньше остальных.
    fn match_expr(&mut self, m: &'a ast::MatchExpr, line: u32, want: Option<&Type>) -> R<Expr> {
        let value = if m.mode == Some(ast::Mode::Mut) {
            let (place, ty) = self.place(
                &m.value,
                "match mut — только у места: переменной, параметра, их поля или элемента",
            )?;
            Expr {
                kind: ExprKind::Place(place),
                ty,
                line: m.value.line,
            }
        } else {
            let v = self.expr(&m.value, None)?;
            // место типа Never сравнивают в match без ветвей: ветвей у него нет, и недостижимого
            // кода тоже (решение 28)
            if matches!(v.kind, ExprKind::Place(_)) {
                v
            } else {
                self.valued(v)?
            }
        };
        // режим, которым сравнивается значение: временное значение образец забирает сам (12.62)
        let mode = match m.mode {
            Some(ast::Mode::Mut) => Mode::Mut,
            Some(ast::Mode::Take) => Mode::Take,
            None if matches!(value.kind, ExprKind::Place(_)) => Mode::Read,
            None => Mode::Take,
        };
        let mut heads: Vec<Head<'a>> = Vec::new();
        for item in &m.items {
            heads.push(match item {
                ast::MatchItem::Case(c) => self.case_head(c, &value.ty, mode)?,
                ast::MatchItem::Else(b, _, at) => Head::Else(b, *at),
            });
        }
        self.coverage(&heads, &value.ty, line)?;
        let blocks: Vec<&'a ast::Block> = heads
            .iter()
            .map(|h| match h {
                Head::Case { body, .. } => *body,
                Head::Else(b, _) => *b,
            })
            .collect();
        let mut ty: Option<Type> = want.cloned();
        let mut checked: Vec<Option<Block>> = vec![None; blocks.len()];
        for k in branch_order(&blocks) {
            // имена ветви видны и в её теле
            let names = match &heads[k] {
                Head::Case { names, .. } => names.clone(),
                Head::Else(..) => Vec::new(),
            };
            self.reopen_scope(&names);
            let b = self.block(blocks[k], ty.as_ref());
            self.close_scope();
            let b = b?;
            if ty.is_none() && b.ty != Type::Never {
                ty = Some(b.ty.clone());
            }
            checked[k] = Some(b);
        }
        let mut cases = Vec::new();
        let mut otherwise = None;
        for (head, body) in heads.into_iter().zip(checked) {
            let body = body.unwrap_or_default();
            match head {
                Head::Case {
                    patterns,
                    guard,
                    line,
                    ..
                } => cases.push(Case {
                    patterns,
                    guard,
                    body,
                    line,
                }),
                // недостижимых ветвей нет, поэтому else — последний и один
                Head::Else(..) => otherwise = Some(body),
            }
        }
        // уходят все ветви — уходит и match; без ветвей он бывает только по значению типа без
        // значений (9.36, решение 28)
        let all_leave = cases.iter().all(|c| c.body.ty == Type::Never)
            && otherwise.as_ref().is_none_or(|b| b.ty == Type::Never);
        let ty = if all_leave {
            Type::Never
        } else {
            ty.unwrap_or(Type::Unit)
        };
        Ok(Expr {
            kind: ExprKind::Match(Box::new(Match {
                value,
                mode,
                cases,
                otherwise,
            })),
            ty,
            line,
        })
    }

    /// Образцы и условие ветви. Первая альтернатива объявляет имена, остальные связывают те же
    /// имена тех же типов (12.43); имена видны в условии, затем область закрывается до тела.
    fn case_head(&mut self, c: &'a ast::MatchCase, ty: &Type, mode: Mode) -> R<Head<'a>> {
        self.scopes.push(Vec::new());
        let mut patterns = Vec::new();
        let mut first: Vec<(&'a str, LocalId)> = Vec::new();
        for (k, p) in c.patterns.iter().enumerate() {
            if k == 0 {
                patterns.push(self.pattern(p, ty, &mut Binds::Declare(mode))?);
                first = match self.scopes.last() {
                    Some(names) => names
                        .iter()
                        .filter_map(|&name| self.visible.get(name).map(|&id| (name, id)))
                        .collect(),
                    None => Vec::new(),
                };
                continue;
            }
            let mut binds = Binds::Same {
                first: &first,
                seen: vec![false; first.len()],
                alternative: k + 1,
            };
            patterns.push(self.pattern(p, ty, &mut binds)?);
            if let Binds::Same { seen, .. } = &binds {
                if let Some(j) = seen.iter().position(|s| !s) {
                    return fail(
                        p.line,
                        format!(
                            "альтернатива {} не связывает имя {}: альтернативы связывают одинаковые имена (12.43)",
                            k + 1,
                            first[j].0
                        ),
                    );
                }
            }
        }
        let guard = match &c.guard {
            Some(g) => Some(self.inner(g, Some(&BOOL))?),
            None => None,
        };
        let names = self.close_scope();
        Ok(Head::Case {
            patterns,
            guard,
            names,
            body: &c.body,
            line: c.patterns.first().map_or(0, |p| p.line),
        })
    }

    /// Недостижимые ветви и полнота (раздел 6, решения 7 и 28, 9.35).
    fn coverage(&self, heads: &[Head<'a>], ty: &Type, line: u32) -> R<()> {
        let rows: Vec<patterns::Row> = heads
            .iter()
            .map(|h| match h {
                Head::Case {
                    patterns, guard, ..
                } => patterns::Row {
                    alternatives: patterns.iter().map(Some).collect(),
                    guarded: guard.is_some(),
                },
                Head::Else(..) => patterns::Row {
                    alternatives: vec![None],
                    guarded: false,
                },
            })
            .collect();
        match patterns::check(self.p, ty, &rows) {
            Ok(()) => Ok(()),
            Err(patterns::Problem::Unreachable {
                row,
                alternative,
                cause,
            }) => {
                let (at, subject) = match (&heads[row], alternative) {
                    (Head::Else(_, at), _) => (*at, Subject::Else),
                    (Head::Case { patterns, .. }, Some(k)) => {
                        (patterns[k].line, Subject::Alternative(k + 1))
                    }
                    (Head::Case { line, .. }, None) => (*line, Subject::Branch),
                };
                fail(at, self.unreachable(heads, ty, row, subject, cause))
            }
            Err(patterns::Problem::Missing(Some(example))) => {
                let guarded = heads
                    .iter()
                    .any(|h| matches!(h, Head::Case { guard: Some(_), .. }));
                let message = if guarded {
                    format!(
                        "match разбирает не все значения: например, {} не подходит ни к одной ветви без условия, а ветви с условием в полноте не участвуют (9.35); добавьте ветвь или else",
                        example
                    )
                } else {
                    format!(
                        "match разбирает не все значения: например, {} не подходит ни к одной ветви (9.35); добавьте ветвь или else",
                        example
                    )
                };
                fail(line, message)
            }
            // пример не нужен: ни одна ветвь без условия ничего не разбирает
            Err(patterns::Problem::Missing(None)) if heads.is_empty() => fail(
                line,
                format!(
                    "match без ветвей бывает только по значению типа без значений, а у {} значения есть (решение 28); добавьте ветвь или else",
                    self.text(ty)
                ),
            ),
            Err(patterns::Problem::Missing(None)) => fail(
                line,
                "у match нет ни одной ветви без условия, а ветви с условием в полноте не участвуют (9.35); добавьте ветвь без условия или else"
                    .to_string(),
            ),
        }
    }

    /// Сообщение о недостижимой ветви `row`, её альтернативе или `else` (решения 7 и 28).
    fn unreachable(
        &self,
        heads: &[Head<'a>],
        ty: &Type,
        row: usize,
        subject: Subject,
        cause: patterns::Cause,
    ) -> String {
        let what = match subject {
            Subject::Branch => "ветвь недостижима".to_string(),
            Subject::Alternative(k) => format!("альтернатива {} недостижима", k),
            Subject::Else => "else недостижим".to_string(),
        };
        match cause {
            patterns::Cause::NoValues(t) => {
                let (to, needless) = match subject {
                    Subject::Branch => ("к ветви".to_string(), "ветвь не нужна"),
                    Subject::Alternative(k) => {
                        (format!("к альтернативе {}", k), "альтернатива не нужна")
                    }
                    Subject::Else => ("к else".to_string(), "else не нужен"),
                };
                format!(
                    "{} не подходит ни одно значение: у типа {} значений нет (решение 28); {}",
                    to,
                    self.text(&t),
                    needless
                )
            }
            patterns::Cause::Covered => match subject {
                Subject::Branch => format!(
                    "{}: все её значения разобраны ветвями выше (решение 7)",
                    what
                ),
                Subject::Alternative(_) => {
                    format!("{}: все её значения разобраны выше (решение 7)", what)
                }
                Subject::Else => {
                    format!("{}: все значения разобраны ветвями выше (решение 7)", what)
                }
            },
            patterns::Cause::CoveredBy {
                row: by,
                alternative,
            } => {
                let p = match &heads[by] {
                    Head::Else(_, at) => {
                        return self.caught(
                            &what,
                            subject,
                            &format!("else в строке {}", at),
                            None,
                            ty,
                        )
                    }
                    Head::Case { patterns, .. } => &patterns[alternative],
                };
                let same = by == row;
                match &p.kind {
                    PatternKind::Skip | PatternKind::Bind(_) => {
                        let by_text = if same {
                            format!("альтернатива {}", alternative + 1)
                        } else {
                            format!("ветвь в строке {}", p.line)
                        };
                        self.caught(&what, subject, &by_text, Some(&p.kind), ty)
                    }
                    _ if same => format!(
                        "{}: все её значения разобраны альтернативой {} (решение 7)",
                        what,
                        alternative + 1
                    ),
                    _ => format!(
                        "{}: все её значения разобраны ветвью в строке {} (решение 7)",
                        what, p.line
                    ),
                }
            }
        }
    }

    /// Сообщение, когда всё разобрала ветвь или альтернатива `by` выше, к которой подходит любое
    /// значение: `_`, имя или `else`.
    fn caught(
        &self,
        what: &str,
        subject: Subject,
        by: &str,
        kind: Option<&PatternKind>,
        ty: &Type,
    ) -> String {
        // имя, которое не вариант, — новое имя, и часто это опечатка в варианте (12.42)
        if let (Some(PatternKind::Bind(id)), Type::Named(e)) = (kind, ty) {
            let decl = &self.p.types[*e];
            if matches!(decl.kind, TypeKind::Enum(_)) {
                let x = &self.locals[*id].name;
                return format!(
                    "{}: {} подходит к любому значению — у {} нет варианта {}, поэтому {} там новое имя (12.42, решение 7)",
                    what, by, decl.name, x, x
                );
            }
        }
        match subject {
            Subject::Branch => format!(
                "{}: {} подходит к любому значению (решение 7); удалите эту ветвь или поставьте её выше",
                what, by
            ),
            Subject::Alternative(_) | Subject::Else => {
                format!("{}: {} подходит к любому значению (решение 7)", what, by)
            }
        }
    }

    /// Образец по значению типа `ty` (раздел 6; 12.42, 12.43, 13.10, 13.11).
    fn pattern(&mut self, p: &'a ast::Pattern, ty: &Type, binds: &mut Binds<'_, 'a>) -> R<Pattern> {
        let line = p.line;
        let kind = match &p.kind {
            ast::PatternKind::Wildcard => PatternKind::Skip,
            ast::PatternKind::Int(negative, text) => {
                if is_float(ty) {
                    return fail(
                        line,
                        format!(
                            "образец-число на месте дробного {}: у дробных нет полного равенства (решение 10з)",
                            self.text(ty)
                        ),
                    );
                }
                PatternKind::Int(self.int_value(text, *negative, line, Some(ty))?.0)
            }
            ast::PatternKind::Float(negative, text) => {
                return fail(
                    line,
                    format!(
                    "дробное число {}{} в образце: у дробных нет полного равенства (решение 10з)",
                    if *negative { "-" } else { "" },
                    text
                ),
                )
            }
            ast::PatternKind::Str(_) => return outside(line, "строки"),
            ast::PatternKind::Char(raw) => {
                if *ty != CHAR {
                    return self.mismatch(line, ty, &CHAR);
                }
                PatternKind::Char(char_value(raw, line)?)
            }
            ast::PatternKind::Bool(b) => {
                if *ty != BOOL {
                    return self.mismatch(line, ty, &BOOL);
                }
                PatternKind::Bool(*b)
            }
            ast::PatternKind::Path(names) => self.path_pattern(names, ty, line, binds)?,
            ast::PatternKind::Ctor(names, args) => {
                self.variant_pattern(names, args, ty, line, binds)?
            }
            ast::PatternKind::Tuple(items) => match ty {
                Type::Tuple(parts) if parts.len() == items.len() => {
                    let mut out = Vec::new();
                    for (item, part) in items.iter().zip(parts) {
                        out.push(self.pattern(item, part, binds)?);
                    }
                    PatternKind::Tuple(out)
                }
                _ => {
                    return fail(
                        line,
                        format!(
                            "образец из {} частей, а значение типа {}",
                            items.len(),
                            self.text(ty)
                        ),
                    )
                }
            },
        };
        Ok(Pattern {
            kind,
            ty: ty.clone(),
            line,
        })
    }

    /// Образец без скобок: вариант без значения или новое имя. Короткое имя — вариант, если это
    /// вариант ожидаемого перечисления, иначе — новое имя (12.42, решение 3).
    fn path_pattern(
        &mut self,
        names: &'a [String],
        ty: &Type,
        line: u32,
        binds: &mut Binds<'_, 'a>,
    ) -> R<PatternKind> {
        match names {
            [name] => match self.variant_of(ty, name) {
                Some((id, k)) => self.unit_variant(id, k, name, line),
                None => Ok(PatternKind::Bind(self.bind(name, ty, line, binds)?)),
            },
            [owner, name] => {
                let (id, k) = self.qualified_variant(owner, name, ty, line)?;
                self.unit_variant(id, k, &format!("{}.{}", owner, name), line)
            }
            _ => fail(line, "в образце вариант пишется V или E.V".to_string()),
        }
    }

    /// Вариант ожидаемого перечисления с таким именем.
    fn variant_of(&self, ty: &Type, name: &str) -> Option<(TypeId, usize)> {
        match ty {
            Type::Named(id) => self.t.variants[*id].get(name).map(|&k| (*id, k)),
            _ => None,
        }
    }

    /// Вариант `E.V` в образце: перечисление — то, которое ждут (решение 3).
    fn qualified_variant(
        &self,
        owner: &str,
        name: &str,
        ty: &Type,
        line: u32,
    ) -> R<(TypeId, usize)> {
        let id = match self.c.names.get(owner) {
            Some((Item::Type(id), _)) => *id,
            _ if builtin_type(owner) || STANDARD_NAMES.contains(&owner) => {
                return fail(line, format!("{} — не перечисление пакета", owner))
            }
            _ => return fail(line, format!("нет перечисления {}", owner)),
        };
        if let TypeKind::Struct(_) = self.p.types[id].kind {
            return fail(line, format!("{} — запись, а не перечисление", owner));
        }
        if *ty != Type::Named(id) {
            return self.mismatch(line, ty, &Type::Named(id));
        }
        match self.t.variants[id].get(name) {
            Some(&k) => Ok((id, k)),
            None => fail(
                line,
                format!("у перечисления {} нет варианта {}", owner, name),
            ),
        }
    }

    /// Вариант без скобок: у него не должно быть значения (12.42).
    fn unit_variant(&self, id: TypeId, k: usize, written: &str, line: u32) -> R<PatternKind> {
        let (full, payload) = self.variant_decl(id, k);
        match payload {
            Some(Payload::None) => Ok(PatternKind::Variant(id, k, Vec::new())),
            _ => fail(
                line,
                format!("у варианта {} есть значение: case {}(…)", full, written),
            ),
        }
    }

    /// Полное имя варианта `E.V` и что он несёт.
    fn variant_decl(&self, id: TypeId, k: usize) -> (String, Option<&'c Payload>) {
        let decl = &self.p.types[id];
        match &decl.kind {
            TypeKind::Enum(variants) => (
                format!("{}.{}", decl.name, variants[k].name),
                Some(&variants[k].payload),
            ),
            TypeKind::Struct(_) => (decl.name.clone(), None),
        }
    }

    /// Образец со скобками: вариант со значением (12.42, 13.11).
    fn variant_pattern(
        &mut self,
        names: &'a [String],
        args: &'a [ast::PatternArg],
        ty: &Type,
        line: u32,
        binds: &mut Binds<'_, 'a>,
    ) -> R<PatternKind> {
        let (id, k, written) = match names {
            [name] => match self.variant_of(ty, name) {
                Some((id, k)) => (id, k, name.clone()),
                None => return self.not_variant(name, ty, line),
            },
            [owner, name] => {
                let (id, k) = self.qualified_variant(owner, name, ty, line)?;
                (id, k, format!("{}.{}", owner, name))
            }
            _ => {
                return fail(
                    line,
                    "в образце вариант пишется V(…) или E.V(…)".to_string(),
                )
            }
        };
        let (full, payload) = self.variant_decl(id, k);
        let items = match payload {
            Some(Payload::One(t)) => match args {
                [ast::PatternArg::Pattern(inner)] => vec![self.pattern(inner, t, binds)?],
                _ => {
                    return fail(
                        line,
                        format!(
                            "вариант {} несёт одно значение без имени: case {}(образец)",
                            full, written
                        ),
                    )
                }
            },
            Some(Payload::Fields(fields)) => {
                self.field_patterns(fields, args, &full, line, binds)?
            }
            _ => {
                return fail(
                    line,
                    format!("у варианта {} нет значения: case {}", full, written),
                )
            }
        };
        Ok(PatternKind::Variant(id, k, items))
    }

    /// Имя со скобками, которое не вариант ожидаемого перечисления.
    fn not_variant<T>(&self, name: &str, ty: &Type, line: u32) -> R<T> {
        if let Some((Item::Type(id), _)) = self.c.names.get(name) {
            if let TypeKind::Struct(_) = self.p.types[*id].kind {
                return outside(line, "образец записи (решение 13)");
            }
        }
        match ty {
            Type::Named(id) if matches!(self.p.types[*id].kind, TypeKind::Enum(_)) => fail(
                line,
                format!(
                    "у перечисления {} нет варианта {}",
                    self.p.types[*id].name, name
                ),
            ),
            _ if matches!(name, "Some" | "None") => outside(line, "Option"),
            _ if matches!(name, "Ok" | "Err") => outside(line, "Result"),
            _ => fail(
                line,
                format!(
                    "образец {}(…) — вариант, а значение типа {}",
                    name,
                    self.text(ty)
                ),
            ),
        }
    }

    /// Образцы именованных полей: в порядке объявления, имя без двоеточия — имя поля, `_`
    /// пропускает одно поле, `..` — остальные и стоит последним (12.42, 13.11).
    fn field_patterns(
        &mut self,
        fields: &'c [Field],
        args: &'a [ast::PatternArg],
        full: &str,
        line: u32,
        binds: &mut Binds<'_, 'a>,
    ) -> R<Vec<Pattern>> {
        // строка части образца; у `..` своей строки нет
        let arg_line = |arg: &ast::PatternArg| match arg {
            ast::PatternArg::Pattern(p) | ast::PatternArg::Field(_, p) => p.line,
            ast::PatternArg::Rest => line,
        };
        let mut out: Vec<Pattern> = Vec::new();
        let mut rest = false;
        for (j, arg) in args.iter().enumerate() {
            let k = out.len();
            match arg {
                ast::PatternArg::Rest => {
                    if let Some(next) = args.get(j + 1) {
                        return fail(arg_line(next), ".. стоит последним (12.42)".to_string());
                    }
                    rest = true;
                }
                _ if k == fields.len() => {
                    return fail(
                        arg_line(arg),
                        format!(
                            "у варианта {} полей: {}, а в образце больше",
                            full,
                            fields.len()
                        ),
                    )
                }
                ast::PatternArg::Pattern(p) => match &p.kind {
                    ast::PatternKind::Wildcard => out.push(Pattern {
                        kind: PatternKind::Skip,
                        ty: fields[k].ty.clone(),
                        line: p.line,
                    }),
                    ast::PatternKind::Path(names) if names.len() == 1 => {
                        if names[0] != fields[k].name {
                            return fail(
                                p.line,
                                format!(
                                    "в образце варианта {} на этом месте поле {}: имя без двоеточия совпадает с именем поля (13.11)",
                                    full, fields[k].name
                                ),
                            );
                        }
                        out.push(self.pattern(p, &fields[k].ty, binds)?);
                    }
                    _ => {
                        let f = &fields[k].name;
                        return fail(
                            p.line,
                            format!(
                                "в образце варианта {} поле {} пишется именем {} или {}: образец (13.11)",
                                full, f, f, f
                            ),
                        );
                    }
                },
                ast::PatternArg::Field(name, p) => {
                    if *name != fields[k].name {
                        let message = if fields.iter().any(|f| f.name == *name) {
                            format!(
                                "в образце варианта {} на этом месте поле {}, а не {}: поля идут в порядке объявления (13.11)",
                                full, fields[k].name, name
                            )
                        } else {
                            format!("у варианта {} нет поля {}", full, name)
                        };
                        return fail(p.line, message);
                    }
                    out.push(self.pattern(p, &fields[k].ty, binds)?);
                }
            }
        }
        if rest {
            for f in &fields[out.len()..] {
                out.push(Pattern {
                    kind: PatternKind::Skip,
                    ty: f.ty.clone(),
                    line,
                });
            }
        } else if let Some(f) = fields.get(out.len()) {
            return fail(
                line,
                format!(
                    "в образце варианта {} нет поля {}: остальные поля пропускает .. (12.42)",
                    full, f.name
                ),
            );
        }
        Ok(out)
    }

    /// Имя из образца. В первой альтернативе оно объявляется, в остальных — то же имя того же
    /// типа (12.43).
    fn bind(
        &mut self,
        name: &'a str,
        ty: &Type,
        line: u32,
        binds: &mut Binds<'_, 'a>,
    ) -> R<LocalId> {
        match binds {
            Binds::Declare(mode) => self.declare(name, ty.clone(), LocalKind::Bound(*mode), line),
            Binds::Same {
                first,
                seen,
                alternative,
            } => {
                let Some(j) = first.iter().position(|&(n, _)| n == name) else {
                    return fail(
                        line,
                        format!(
                            "имя {} связывает альтернатива {}, а первая — нет: альтернативы связывают одинаковые имена (12.43)",
                            name, alternative
                        ),
                    );
                };
                if seen[j] {
                    return fail(
                        line,
                        format!(
                            "имя {} в альтернативе {} связано дважды (12.93)",
                            name, alternative
                        ),
                    );
                }
                seen[j] = true;
                let id = first[j].1;
                if self.locals[id].ty != *ty {
                    return fail(
                        line,
                        format!(
                            "имя {} в альтернативе {} типа {}, а в первой — {}: альтернативы связывают одинаковые типы (12.43)",
                            name,
                            alternative,
                            self.text(ty),
                            self.text(&self.locals[id].ty)
                        ),
                    );
                }
                Ok(id)
            }
        }
    }

    // ───────── имена в выражениях, вызовы, поля и элементы ─────────

    /// Имя и то, что к нему приписано: поля, элементы, вызов.
    fn postfix(&mut self, e: &'a ast::Expr) -> R<Expr> {
        let (base, ops): (&'a ast::Expr, &'a [ast::PostfixOp]) = match &e.kind {
            ast::ExprKind::Postfix(base, ops) => (base, ops),
            _ => (e, &[]),
        };
        let mut used = 0;
        let mut cur = match &base.kind {
            ast::ExprKind::Name(name) => {
                let (v, k) = self.name_value(name, base.line, ops)?;
                used = k;
                v
            }
            _ => self.inner(base, None)?,
        };
        for (k, op) in ops.iter().enumerate().skip(used) {
            // поле, элемент или вызов берут у значения (решение 14)
            cur = self.valued(cur)?;
            cur = match op {
                ast::PostfixOp::Field(name) => {
                    if matches!(ops.get(k + 1), Some(ast::PostfixOp::Call(_))) {
                        return outside(cur.line, "методы");
                    }
                    self.field(cur, name)?
                }
                ast::PostfixOp::Index(args) => self.index(cur, args)?,
                ast::PostfixOp::Call(_) => {
                    return fail(
                        cur.line,
                        format!(
                            "здесь вызывают значение типа {}, а вызвать можно только функцию",
                            self.text(&cur.ty)
                        ),
                    )
                }
            };
        }
        Ok(cur)
    }

    /// Имя что-то называет: локальное имя, объявление пакета, встроенный тип, стандартное имя
    /// или вариант какого-нибудь перечисления.
    fn names_something(&self, name: &str) -> bool {
        self.visible.contains_key(name)
            || self.c.names.contains_key(name)
            || builtin_type(name)
            || STANDARD_NAMES.contains(&name)
            || self.t.variant_owner.contains_key(name)
    }

    /// Значение имени и сколько приписанного к нему оно забрало: вызов функции, создание записи
    /// или варианта, преобразование числа.
    fn name_value(
        &mut self,
        name: &'a str,
        line: u32,
        ops: &'a [ast::PostfixOp],
    ) -> R<(Expr, usize)> {
        if let Some(&local) = self.visible.get(name) {
            let ty = self.locals[local].ty.clone();
            return Ok((
                Expr {
                    kind: ExprKind::Place(Place {
                        base: local,
                        path: Vec::new(),
                    }),
                    ty,
                    line,
                },
                0,
            ));
        }
        match self.c.names.get(name) {
            Some((Item::Func(id), _)) => {
                return match ops.first() {
                    Some(ast::PostfixOp::Call(args)) => Ok((self.call(*id, args, line)?, 1)),
                    Some(ast::PostfixOp::Index(_)) => outside(line, "обобщения"),
                    _ => outside(line, "функции как значения"),
                };
            }
            Some((Item::Type(id), _)) => return self.type_value(*id, line, ops),
            None => {}
        }
        if let Some(scalar) = Scalar::from_name(name) {
            return match ops.first() {
                Some(ast::PostfixOp::Call(args)) => Ok((self.convert(scalar, args, line)?, 1)),
                _ => fail(line, format!("{} — тип, а не значение", name)),
            };
        }
        if builtin_type(name) {
            return fail(line, format!("{} — тип, а не значение", name));
        }
        match name {
            "Some" | "None" => return outside(line, "Option"),
            "Ok" | "Err" => return outside(line, "Result"),
            _ => {}
        }
        // вариант своего перечисления раньше стандартного имени: `Add` скорее значит `Op.Add`,
        // чем интерфейс сложения
        if let Some(&owner) = self.t.variant_owner.get(name) {
            return fail(
                line,
                format!(
                    "вариант пишется с именем типа: {}.{} (12.89)",
                    self.p.types[owner].name, name
                ),
            );
        }
        if STANDARD_INTERFACES.contains(&name) {
            return fail(line, format!("{} — интерфейс, а не значение", name));
        }
        if STANDARD_NAMES.contains(&name) {
            return outside(line, name);
        }
        fail(line, format!("нет имени {}", name))
    }

    /// Имя записи или перечисления в выражении: создание записи `P(…)` или варианта `E.V(…)`.
    fn type_value(&mut self, id: TypeId, line: u32, ops: &'a [ast::PostfixOp]) -> R<(Expr, usize)> {
        if let Some(ast::PostfixOp::Index(_)) = ops.first() {
            return outside(line, "обобщения");
        }
        let decl = &self.p.types[id];
        match &decl.kind {
            TypeKind::Struct(fields) => match ops.first() {
                Some(ast::PostfixOp::Call(args)) => Ok((self.record(id, fields, args, line)?, 1)),
                _ => fail(
                    line,
                    format!(
                        "{} — тип, а не значение: запись создаётся вызовом {}(…)",
                        decl.name, decl.name
                    ),
                ),
            },
            TypeKind::Enum(variants) => {
                let Some(ast::PostfixOp::Field(vname)) = ops.first() else {
                    return fail(
                        line,
                        format!(
                            "{} — тип, а не значение: вариант пишется {}.Вариант",
                            decl.name, decl.name
                        ),
                    );
                };
                let Some(&k) = self.t.variants[id].get(vname.as_str()) else {
                    return fail(
                        line,
                        format!("у перечисления {} нет варианта {}", decl.name, vname),
                    );
                };
                let args = match ops.get(1) {
                    Some(ast::PostfixOp::Call(args)) => Some(args),
                    _ => None,
                };
                let (value, used) = match (&variants[k].payload, args) {
                    (Payload::None, None) => (VariantValue::None, 1),
                    (Payload::None, Some(_)) => {
                        return fail(
                            line,
                            format!("у варианта {}.{} нет значения", decl.name, vname),
                        )
                    }
                    (Payload::One(_), None) | (Payload::Fields(_), None) => {
                        return fail(
                            line,
                            format!(
                                "у варианта {}.{} есть значение: {}.{}(…)",
                                decl.name, vname, decl.name, vname
                            ),
                        )
                    }
                    (Payload::One(ty), Some(args)) => {
                        let ty = ty.clone();
                        match args.as_slice() {
                            [ast::Arg::Plain(e)] => {
                                (VariantValue::One(Box::new(self.inner(e, Some(&ty))?)), 2)
                            }
                            _ => {
                                return fail(
                                    line,
                                    format!(
                                    "вариант {}.{} несёт одно значение без имени: {}.{}(значение)",
                                    decl.name, vname, decl.name, vname
                                ),
                                )
                            }
                        }
                    }
                    (Payload::Fields(fields), Some(args)) => {
                        let what = format!("вариант {}.{}", decl.name, vname);
                        let names = &self.t.variant_fields[id][k];
                        (
                            VariantValue::Fields(
                                self.named_fields(fields, names, args, &what, line)?,
                            ),
                            2,
                        )
                    }
                };
                Ok((
                    Expr {
                        kind: ExprKind::Variant(id, k, value),
                        ty: Type::Named(id),
                        line,
                    },
                    used,
                ))
            }
        }
    }

    fn record(&mut self, id: TypeId, fields: &[Field], args: &'a [ast::Arg], line: u32) -> R<Expr> {
        let what = format!("запись {}", self.p.types[id].name);
        let values = self.named_fields(fields, &self.t.fields[id], args, &what, line)?;
        Ok(Expr {
            kind: ExprKind::Record(id, values),
            ty: Type::Named(id),
            line,
        })
    }

    /// Поля по именам: все, каждое один раз, в любом порядке (решения 10е и 19).
    fn named_fields(
        &mut self,
        fields: &[Field],
        names: &HashMap<&str, usize>,
        args: &'a [ast::Arg],
        what: &str,
        line: u32,
    ) -> R<Vec<(usize, Expr)>> {
        let mut seen = vec![false; fields.len()];
        let mut out = Vec::new();
        for arg in args {
            let (name, value) = match arg {
                ast::Arg::Named(name, value) => (name, value),
                ast::Arg::Plain(_) | ast::Arg::Mut(_) => {
                    return fail(
                        line,
                        format!("{} создаётся с именами полей: поле: значение (12.35)", what),
                    )
                }
            };
            let Some(&k) = names.get(name.as_str()) else {
                return fail(line, format!("у {} нет поля {}", genitive(what), name));
            };
            if seen[k] {
                return fail(line, format!("поле {} задано дважды", name));
            }
            seen[k] = true;
            let ty = fields[k].ty.clone();
            out.push((k, self.inner(value, Some(&ty))?));
        }
        if let Some(k) = seen.iter().position(|s| !s) {
            return fail(
                line,
                format!("у {} не задано поле {}", genitive(what), fields[k].name),
            );
        }
        Ok(out)
    }

    /// Преобразование числа `T(x)`: между целыми, дробными и символами, кроме символа и
    /// дробного друг в друга; `bool` не преобразуется (12.57, 9.16–9.18).
    fn convert(&mut self, to: Scalar, args: &'a [ast::Arg], line: u32) -> R<Expr> {
        let [ast::Arg::Plain(arg)] = args else {
            return fail(
                line,
                format!("преобразование {}(…) — один аргумент без имени", to.name()),
            );
        };
        let value = self.inner(arg, None)?;
        let from = match &value.ty {
            Type::Scalar(s) => *s,
            other => {
                return fail(
                    line,
                    format!(
                        "в {} преобразуются только числа и символы, а здесь {}",
                        to.name(),
                        self.text(other)
                    ),
                )
            }
        };
        let kind = |s: Scalar| match s {
            Scalar::Bool => 0,
            Scalar::Char => 1,
            Scalar::F32 | Scalar::F64 => 2,
            _ => 3,
        };
        // bool ни во что не преобразуется, символ и дробное — друг в друга
        let ok = !matches!((kind(from), kind(to)), (0, _) | (_, 0) | (1, 2) | (2, 1));
        if !ok {
            return fail(
                line,
                format!(
                    "{} не преобразуется в {} (9.17, 9.18)",
                    from.name(),
                    to.name()
                ),
            );
        }
        Ok(Expr {
            kind: ExprKind::Convert(to, Box::new(value)),
            ty: Type::Scalar(to),
            line,
        })
    }

    /// Прямой вызов функции пакета: аргументов столько, сколько параметров; у параметра `mut`
    /// аргумент — место с `mut` (12.59), у остальных — без `mut`; именованных нет (12.36).
    fn call(&mut self, id: FuncId, args: &'a [ast::Arg], line: u32) -> R<Expr> {
        let f = &self.p.funcs[id];
        if args.len() != f.params.len() {
            return fail(
                line,
                format!(
                    "функция {} ждёт аргументов: {}, а здесь {}",
                    f.name,
                    f.params.len(),
                    args.len()
                ),
            );
        }
        let mut out = Vec::new();
        for (arg, param) in args.iter().zip(&f.params) {
            let value = match (arg, param.mode) {
                (ast::Arg::Named(..), _) => {
                    return fail(
                        line,
                        "именованных аргументов у функций нет (12.36)".to_string(),
                    )
                }
                (ast::Arg::Mut(e), Mode::Mut) => {
                    let (place, ty) = self.place(
                        e,
                        "с mut отдают место: переменную, параметр, их поле или элемент",
                    )?;
                    if ty != param.ty {
                        return self.mismatch(e.line, &param.ty, &ty);
                    }
                    Expr {
                        kind: ExprKind::Place(place),
                        ty,
                        line: e.line,
                    }
                }
                (ast::Arg::Mut(e), _) => {
                    return fail(
                        e.line,
                        format!(
                            "параметр {} функции {} — не mut: mut при вызове не пишется",
                            param.name, f.name
                        ),
                    )
                }
                (ast::Arg::Plain(e), Mode::Mut) => {
                    return fail(
                        e.line,
                        format!(
                            "параметр {} функции {} — mut: аргумент пишется с mut (12.59)",
                            param.name, f.name
                        ),
                    )
                }
                (ast::Arg::Plain(e), _) => self.inner(e, Some(&param.ty))?,
            };
            out.push(Arg {
                mode: param.mode,
                value,
            });
        }
        Ok(Expr {
            kind: ExprKind::Call(id, out),
            ty: f.result.clone(),
            line,
        })
    }

    fn field(&mut self, cur: Expr, name: &str) -> R<Expr> {
        let line = cur.line;
        let Type::Named(id) = cur.ty else {
            return fail(line, format!("у типа {} нет полей", self.text(&cur.ty)));
        };
        let decl = &self.p.types[id];
        let TypeKind::Struct(fields) = &decl.kind else {
            return fail(
                line,
                format!(
                    "значение варианта берут образцом: у перечисления {} нет полей",
                    decl.name
                ),
            );
        };
        let Some(&k) = self.t.fields[id].get(name) else {
            return fail(line, format!("у записи {} нет поля {}", decl.name, name));
        };
        let ty = fields[k].ty.clone();
        Ok(step(cur, Step::Field(k), ty))
    }

    fn index(&mut self, cur: Expr, args: &'a [ast::IndexArg]) -> R<Expr> {
        let line = cur.line;
        for arg in args {
            match arg {
                ast::IndexArg::Expr(_) => {}
                ast::IndexArg::Any(_) => return outside(line, "any"),
                ast::IndexArg::Uses(_) => return outside(line, "uses"),
            }
        }
        let Type::Array(item, _) = &cur.ty else {
            return fail(
                line,
                format!(
                    "номер берут только у массива, а здесь {}",
                    self.text(&cur.ty)
                ),
            );
        };
        let item = (**item).clone();
        let [ast::IndexArg::Expr(i)] = args else {
            return fail(line, "у элемента массива один номер: a[i]".to_string());
        };
        let i = self.inner(i, Some(&I64))?;
        Ok(step(cur, Step::Elem(Box::new(i)), item))
    }
}

/// Шаг пути к части значения: у места продолжается его путь, у другого значения — путь его
/// части `Part`. Так путь не вкладывается сам в себя.
fn step(cur: Expr, step: Step, ty: Type) -> Expr {
    let line = cur.line;
    let kind = match cur.kind {
        ExprKind::Place(mut place) => {
            place.path.push(step);
            ExprKind::Place(place)
        }
        ExprKind::Part(base, mut path) => {
            path.push(step);
            ExprKind::Part(base, path)
        }
        kind => ExprKind::Part(
            Box::new(Expr {
                kind,
                ty: cur.ty,
                line,
            }),
            vec![step],
        ),
    };
    Expr { kind, ty, line }
}

/// Операция определена для операндов этого типа (раздел 4, «Операции»).
fn op_ok(op: BinOp, ty: &Type) -> bool {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => is_number(ty),
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => is_int(ty),
        BinOp::Eq | BinOp::Ne => matches!(ty, Type::Scalar(_)),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            is_number(ty) || *ty == Type::Scalar(Scalar::Char)
        }
    }
}

fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
    }
}

/// «запись P» → «записи P», «вариант E.V» → «варианта E.V».
fn genitive(what: &str) -> String {
    if let Some(rest) = what.strip_prefix("запись ") {
        format!("записи {}", rest)
    } else if let Some(rest) = what.strip_prefix("вариант ") {
        format!("варианта {}", rest)
    } else {
        what.to_string()
    }
}
