//! Разборщик Nulix: рекурсивный спуск по принятой грамматике `grammar/nulix.lark`.
//!
//! Функции названы по правилам грамматики, которые они разбирают. Источник истины — грамматика:
//! тест `tests/parser_golden.rs` сверяет этот разборщик с проверочным (Lark) на общих примерах,
//! и оба обязаны принимать и отвергать одни и те же тексты и строить одинаковые деревья.
//!
//! Разборщик останавливается на первой ошибке. Совпадение меток `end` с именами он не проверяет:
//! этого нет в грамматике, проверка появится на следующем шаге.
//!
//! Вложенность ограничена (`MAX_DEPTH`), поэтому глубина рекурсии разборщика тоже ограничена.
//! Стека по умолчанию для неё может не хватить: вызывать разбор нужно через `stack::run`.

use crate::ast::*;
use crate::dump;
use crate::lexer::{ident_cont, ident_start, lex};
use crate::token::{Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub message: String,
}

type Res<T> = Result<T, ParseError>;

/// Предел вложенности. Уровень добавляют открытая скобка, приставка перед выражением или типом,
/// тип результата после `->`, выражение-заголовок (`if`, `match`, `while`, `for`, `provide`,
/// `limit`, условие в `case`) и выражение в `${…}`.
pub const MAX_DEPTH: u32 = 256;

const LEVELS: &[&str] = &["@high", "@system", "@low", "@naked"];
const STRICTNESS: &[&str] = &["@prototype", "@develop", "@production", "@strict"];

/// Разбирает целый файл.
pub fn parse_file(source: &str) -> Res<File> {
    let mut parser = Parser::new(source, true, 1)?;
    let file = parser.file()?;
    parser.finish()?;
    Ok(file)
}

/// Разбирает текст с заданного правила грамматики и возвращает каноническую запись дерева.
/// Нужна для сверки с проверочным разборщиком: правила — те же, с которых умеет начинать он.
pub fn parse_rule(rule: &str, source: &str) -> Res<String> {
    let eof_term = rule == "file" || rule == "stmts";
    let mut p = Parser::new(source, eof_term, 1)?;
    let text = match rule {
        "file" => dump::file(&p.file()?),
        "decl" => dump::decl(&p.decl()?),
        "stmt" => dump::stmt(&p.stmt()?),
        "stmts" => dump::stmts(&p.stmt_list()?),
        "expr" => dump::expr(&p.expr()?),
        "type" => dump::ty(&p.ty()?),
        "pattern" => dump::pattern(&p.pattern()?),
        "func_sig" => dump::func_sig(&p.func_sig()?),
        "generics" => dump::generics(&p.generics()?),
        "generic_args" => dump::generic_args(&p.generic_args()?),
        "reqs" => dump::reqs(&p.reqs()?),
        "match_item" => dump::match_item(&p.match_item()?),
        "import_decl" => dump::import(&p.import_decl()?),
        "attr" => dump::attr(&p.attr()?),
        "sig_tail" => {
            if !p.at("->") && !p.at_reqs() {
                return p.fail("результат или требования");
            }
            let (ret, reqs) = p.sig_tail()?;
            dump::sig_tail(&ret, &reqs)
        }
        "param" => dump::param(&p.param()?),
        "field" => dump::field(&p.field()?),
        "variant" => dump::variant(&p.variant()?),
        "bound" => dump::bound(&p.bound()?),
        "file_header" => dump::header(&p.file_header()?),
        "member" => dump::member(&p.member()?),
        _ => {
            return Err(ParseError {
                line: 1,
                message: format!("неизвестное правило грамматики: {}", rule),
            })
        }
    };
    p.finish()?;
    Ok(text)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// Текущая вложенность, см. `MAX_DEPTH`.
    depth: u32,
    /// Вложенность, с которой начался разбор: ноль, а у выражения из `${…}` — уровень внутри строки.
    base: u32,
}

/// Цепочка операций: первый операнд и пары «операция, операнд». Без пар — сам операнд.
fn chain(first: Expr, rest: Vec<(BinOp, Expr)>) -> Expr {
    if rest.is_empty() {
        return first;
    }
    let line = first.line;
    Expr {
        kind: ExprKind::Binary(Box::new(first), rest),
        line,
    }
}

fn too_deep(line: u32) -> ParseError {
    ParseError {
        line,
        message: format!("вложенность глубже {} уровней", MAX_DEPTH),
    }
}

/// Может ли выражение стоять слева от знака присваивания по грамматике: там допустима
/// только цепочка из имени, скобок, литерала или блока с точками, вызовами и индексами.
fn is_postfix_form(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Binary(_, _)
        | ExprKind::Unary(_, _)
        | ExprKind::Is(_, _)
        | ExprKind::Catch(_) => false,
        _ => true,
    }
}

/// Выражение из подстановки `${…}`; `line` — строка, на которой подстановка стоит,
/// `depth` — вложенность, с которой выражение начинается.
fn embedded_expr(text: &str, line: u32, depth: u32) -> Res<Expr> {
    let mut parser = Parser::new(text, false, line)?;
    parser.depth = depth;
    parser.base = depth;
    let value = parser.expr()?;
    parser.finish()?;
    Ok(value)
}

/// Подстановки внутри строкового литерала: `$имя` и `${выражение}`.
/// Запись экранирования и парность скобок в `${…}` уже проверил лексер.
/// Выражение в `${…}` — на уровень глубже строки, `depth` — вложенность самой строки.
fn string_subs(raw: &str, first_line: u32, depth: u32) -> Res<Vec<StrSub>> {
    let chars: Vec<char> = raw.chars().collect();
    let quote = if raw.starts_with("\"\"\"") { 3 } else { 1 };
    let end = chars.len() - quote;
    let mut subs = Vec::new();
    let mut line = first_line;
    let mut i = quote;
    while i < end {
        let c = chars[i];
        if c == '\\' {
            if i + 1 < end && chars[i + 1] == 'u' {
                while i < end && chars[i] != '}' {
                    i += 1;
                }
                i += 1;
            } else {
                i += 2;
            }
        } else if c == '$' && i + 1 < end && chars[i + 1] == '{' {
            let mut braces = 1;
            let mut j = i + 2;
            while j < end && braces > 0 {
                if chars[j] == '{' {
                    braces += 1;
                } else if chars[j] == '}' {
                    braces -= 1;
                }
                j += 1;
            }
            let inner: String = chars[i + 2..j - 1].iter().collect();
            if depth >= MAX_DEPTH {
                return Err(too_deep(line));
            }
            subs.push(StrSub::Expr(embedded_expr(&inner, line, depth + 1)?));
            for k in i..j {
                if chars[k] == '\n' {
                    line += 1;
                }
            }
            i = j;
        } else if c == '$' && i + 1 < end && ident_start(chars[i + 1]) {
            let mut j = i + 2;
            while j < end && ident_cont(chars[j]) {
                j += 1;
            }
            subs.push(StrSub::Name(chars[i + 1..j].iter().collect()));
            i = j;
        } else {
            if c == '\n' {
                line += 1;
            }
            i += 1;
        }
    }
    Ok(subs)
}

impl Parser {
    /// Лексемы текста; `first_line` — номер строки, с которой текст начинается.
    fn new(source: &str, eof_term: bool, first_line: u32) -> Res<Parser> {
        let mut toks = match lex(source, eof_term) {
            Ok(toks) => toks,
            Err(e) => {
                return Err(ParseError {
                    line: e.line + first_line - 1,
                    message: e.message,
                })
            }
        };
        if first_line > 1 {
            for t in toks.iter_mut() {
                t.line += first_line - 1;
            }
        }
        Ok(Parser {
            toks,
            pos: 0,
            depth: 0,
            base: 0,
        })
    }

    // ───────── лексемы ─────────

    /// Стоит ли через `k` лексем ключевое слово, пометка или знак с данным написанием.
    fn is(&self, k: usize, text: &str) -> bool {
        match self.toks.get(self.pos + k) {
            Some(t) => {
                (t.kind == TokenKind::Keyword || t.kind == TokenKind::Punct) && t.text == text
            }
            None => false,
        }
    }

    fn at(&self, text: &str) -> bool {
        self.is(0, text)
    }

    fn kind_is(&self, k: usize, kind: TokenKind) -> bool {
        match self.toks.get(self.pos + k) {
            Some(t) => t.kind == kind,
            None => false,
        }
    }

    fn at_name(&self) -> bool {
        self.kind_is(0, TokenKind::Name)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn at_level(&self) -> bool {
        self.kind_is(0, TokenKind::Keyword) && LEVELS.contains(&self.toks[self.pos].text.as_str())
    }

    fn at_strictness(&self) -> bool {
        self.kind_is(0, TokenKind::Keyword)
            && STRICTNESS.contains(&self.toks[self.pos].text.as_str())
    }

    fn at_marker(&self) -> bool {
        self.at_level() || self.at_strictness()
    }

    /// Съедает лексему, если она здесь стоит. Закрывающая скобка уменьшает вложенность;
    /// открывающие скобки читает `open`, а не эта функция.
    fn eat(&mut self, text: &str) -> bool {
        if self.at(text) {
            self.pos += 1;
            if text == ")" || text == "]" || text == "}" {
                self.shallower();
            }
            true
        } else {
            false
        }
    }

    /// Открывающая скобка, если она здесь стоит: вложенность растёт на уровень.
    fn open(&mut self, text: &str) -> Res<bool> {
        if !self.at(text) {
            return Ok(false);
        }
        self.deeper()?;
        self.pos += 1;
        Ok(true)
    }

    /// Вход на уровень глубже; за пределом вложенности — ошибка.
    fn deeper(&mut self) -> Res<()> {
        if self.depth >= MAX_DEPTH {
            return Err(too_deep(self.line()));
        }
        self.depth += 1;
        Ok(())
    }

    /// Выход с уровня. Каждому выходу отвечает вход, поэтому ниже начала счётчик не уходит;
    /// отладочная сборка это проверяет.
    fn shallower(&mut self) {
        debug_assert!(self.depth > self.base, "выход с уровня без входа");
        self.depth = self.depth.saturating_sub(1);
    }

    fn eat_term(&mut self) -> bool {
        if self.kind_is(0, TokenKind::Term) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Строка текущей лексемы, а в конце текста — последней.
    fn line(&self) -> u32 {
        if self.pos < self.toks.len() {
            self.toks[self.pos].line
        } else if self.toks.is_empty() {
            1
        } else {
            self.toks[self.toks.len() - 1].line
        }
    }

    /// Написание текущей лексемы и шаг вперёд. Вызывается, когда лексема точно есть.
    fn take(&mut self) -> String {
        let text = self.toks[self.pos].text.clone();
        self.pos += 1;
        text
    }

    fn fail<T>(&self, expected: &str) -> Res<T> {
        let found = match self.toks.get(self.pos) {
            None => String::from("конец текста"),
            Some(t) => {
                if t.kind == TokenKind::Term {
                    String::from("конец инструкции")
                } else {
                    format!("«{}»", t.text)
                }
            }
        };
        Err(ParseError {
            line: self.line(),
            message: format!("ожидалось: {}; получено: {}", expected, found),
        })
    }

    fn expect(&mut self, text: &str) -> Res<()> {
        let found = if text == "(" || text == "[" || text == "{" {
            self.open(text)?
        } else {
            self.eat(text)
        };
        if found {
            Ok(())
        } else {
            self.fail(&format!("«{}»", text))
        }
    }

    fn expect_term(&mut self) -> Res<()> {
        if self.eat_term() {
            Ok(())
        } else {
            self.fail("конец инструкции")
        }
    }

    fn name(&mut self) -> Res<String> {
        if self.at_name() {
            Ok(self.take())
        } else {
            self.fail("имя")
        }
    }

    /// Текст должен кончиться. К этому месту счётчик вложенности обязан вернуться к началу;
    /// отладочная сборка это проверяет.
    fn finish(&self) -> Res<()> {
        if self.at_end() {
            debug_assert_eq!(
                self.depth, self.base,
                "счётчик вложенности не вернулся к началу"
            );
            Ok(())
        } else {
            self.fail("конец текста")
        }
    }

    /// Имена через точку: `NAME ("." NAME)*`.
    fn dotted(&mut self) -> Res<Vec<String>> {
        let mut names = Vec::new();
        names.push(self.name()?);
        while self.eat(".") {
            names.push(self.name()?);
        }
        Ok(names)
    }

    fn string(&mut self) -> Res<StrLit> {
        if !self.kind_is(0, TokenKind::Str) {
            return self.fail("строка");
        }
        let line = self.line();
        let raw = self.take();
        let subs = string_subs(&raw, line, self.depth)?;
        Ok(StrLit { raw, subs, line })
    }

    fn mode(&mut self) -> Option<Mode> {
        if self.eat("mut") {
            Some(Mode::Mut)
        } else if self.eat("take") {
            Some(Mode::Take)
        } else {
            None
        }
    }

    /// Необязательная метка `end слово` после блока.
    fn end_kw(&mut self, word: &str) -> Res<bool> {
        if !self.eat("end") {
            return Ok(false);
        }
        self.expect(word)?;
        Ok(true)
    }

    /// Имя в метке после `end`. Слово `test` сразу после `end` лексер выдаёт ключевым,
    /// поэтому грамматика принимает его здесь отдельно.
    fn label_word(&mut self) -> Res<String> {
        if self.at("test") {
            return Ok(self.take());
        }
        self.name()
    }

    /// Необязательная метка `end имя`.
    fn end_name(&mut self) -> Res<Option<String>> {
        if !self.eat("end") {
            return Ok(None);
        }
        Ok(Some(self.label_word()?))
    }

    // ───────── файл ─────────

    fn file(&mut self) -> Res<File> {
        let mut header = None;
        if self.at("@file") {
            header = Some(self.file_header()?);
            self.expect_term()?;
        }
        let mut imports = Vec::new();
        while self.at("import") {
            imports.push(self.import_decl()?);
            self.expect_term()?;
        }
        let mut items = Vec::new();
        while !self.at_end() {
            items.push(self.top_item()?);
            self.expect_term()?;
        }
        Ok(File {
            header,
            imports,
            items,
        })
    }

    fn file_header(&mut self) -> Res<Vec<HeaderArg>> {
        self.expect("@file")?;
        self.expect("(")?;
        let mut args = Vec::new();
        args.push(self.header_arg()?);
        while self.eat(",") {
            args.push(self.header_arg()?);
        }
        self.expect(")")?;
        Ok(args)
    }

    fn header_arg(&mut self) -> Res<HeaderArg> {
        if self.eat("target") {
            self.expect("(")?;
            let name = self.name()?;
            self.expect(")")?;
            return Ok(HeaderArg::Target(name));
        }
        Ok(HeaderArg::Name(self.name()?))
    }

    fn import_decl(&mut self) -> Res<Import> {
        let line = self.line();
        self.expect("import")?;
        let mut path = Vec::new();
        path.push(self.name()?);
        while self.at(".") && self.kind_is(1, TokenKind::Name) {
            self.pos += 1;
            path.push(self.take());
        }
        if self.eat(".") {
            self.expect("{")?;
            let mut names = Vec::new();
            names.push(self.import_name()?);
            while self.eat(",") {
                if self.at("}") {
                    break;
                }
                names.push(self.import_name()?);
            }
            self.expect("}")?;
            return Ok(Import {
                path,
                tail: ImportTail::Names(names),
                line,
            });
        }
        let alias = if self.eat("as") {
            Some(self.name()?)
        } else {
            None
        };
        Ok(Import {
            path,
            tail: ImportTail::Module(alias),
            line,
        })
    }

    fn import_name(&mut self) -> Res<ImportName> {
        let name = self.name()?;
        let alias = if self.eat("as") {
            Some(self.name()?)
        } else {
            None
        };
        Ok(ImportName { name, alias })
    }

    fn top_item(&mut self) -> Res<TopItem> {
        if self.starts_decl() {
            Ok(TopItem::Decl(self.decl()?))
        } else {
            Ok(TopItem::Stmt(self.plain_stmt()?))
        }
    }

    /// Начинается ли здесь объявление. Пометка уровня или строгости перед `{` — это блок-выражение.
    fn starts_decl(&self) -> bool {
        if self.kind_is(0, TokenKind::Attr) || self.at("pub") {
            return true;
        }
        if self.at_marker() {
            return !self.is(1, "{");
        }
        self.at("let")
            || self.at("var")
            || self.at("const")
            || (self.at("func") && self.kind_is(1, TokenKind::Name))
            || self.at("extern")
            || self.at("struct")
            || self.at("enum")
            || self.at("interface")
            || self.at("impl")
            || self.at("type")
            || self.at("test")
    }

    // ───────── объявления ─────────

    fn decl(&mut self) -> Res<Decl> {
        let line = self.line();
        let attrs = self.attrs()?;
        let is_pub = self.eat("pub");
        let kind = self.decl_core()?;
        Ok(Decl {
            attrs,
            is_pub,
            kind,
            line,
        })
    }

    /// Пометки перед объявлением, полем или членом; после каждой может стоять конец инструкции.
    fn attrs(&mut self) -> Res<Vec<Attr>> {
        let mut list = Vec::new();
        while self.kind_is(0, TokenKind::Attr) || self.at_marker() {
            list.push(self.attr()?);
            self.eat_term();
        }
        Ok(list)
    }

    fn attr(&mut self) -> Res<Attr> {
        if self.at_level() {
            return Ok(Attr::Level(self.take()));
        }
        if self.at_strictness() {
            return Ok(Attr::Strictness(self.take()));
        }
        if !self.kind_is(0, TokenKind::Attr) {
            return self.fail("пометка");
        }
        let name = self.take();
        if self.open("(")? {
            let args = self.args()?;
            self.expect(")")?;
            return Ok(Attr::Custom(name, Some(args)));
        }
        Ok(Attr::Custom(name, None))
    }

    fn decl_core(&mut self) -> Res<DeclKind> {
        if self.at("let") || self.at("var") || self.at("const") {
            return Ok(DeclKind::Binding(self.binding()?));
        }
        if self.at("func") {
            return Ok(DeclKind::Func(self.func_decl(None)?));
        }
        if self.eat("extern") {
            let abi = self.string()?;
            if self.at("trust") {
                return Ok(DeclKind::ExternBlock(self.extern_block(abi)?));
            }
            return Ok(DeclKind::Func(self.func_decl(Some(abi))?));
        }
        if self.at("struct") {
            return Ok(DeclKind::Struct(self.struct_decl()?));
        }
        if self.at("enum") {
            return Ok(DeclKind::Enum(self.enum_decl()?));
        }
        if self.at("interface") {
            return Ok(DeclKind::Interface(self.interface_decl()?));
        }
        if self.at("impl") {
            return Ok(DeclKind::Impl(self.impl_decl()?));
        }
        if self.at("type") {
            return Ok(DeclKind::TypeAlias(self.type_decl()?));
        }
        if self.at("test") {
            return Ok(DeclKind::Test(self.test_decl()?));
        }
        self.fail("объявление")
    }

    fn binding(&mut self) -> Res<Binding> {
        let line = self.line();
        let kind = if self.eat("let") {
            BindKind::Let
        } else if self.eat("var") {
            BindKind::Var
        } else if self.eat("const") {
            BindKind::Const
        } else {
            return self.fail("«let», «var» или «const»");
        };
        let target = if self.at("(") {
            BindTarget::Tuple(self.tuple_pattern()?)
        } else {
            BindTarget::Name(self.name()?)
        };
        let ty = if self.eat(":") {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect("=")?;
        let value = self.expr()?;
        Ok(Binding {
            kind,
            target,
            ty,
            value,
            line,
        })
    }

    /// Функция с телом: `func_decl` и `extern_func` (тогда `abi` уже прочитан).
    fn func_decl(&mut self, abi: Option<StrLit>) -> Res<FuncDecl> {
        let sig = self.func_sig()?;
        let body = self.block()?;
        let end = self.end_name()?;
        Ok(FuncDecl {
            abi,
            sig,
            body,
            end,
        })
    }

    fn func_sig(&mut self) -> Res<FuncSig> {
        let line = self.line();
        self.expect("func")?;
        let name = self.name()?;
        let generics = if self.at("[") {
            self.generics()?
        } else {
            Vec::new()
        };
        self.expect("(")?;
        let params = self.params()?;
        self.expect(")")?;
        let (ret, reqs) = self.sig_tail()?;
        Ok(FuncSig {
            name,
            generics,
            params,
            ret,
            reqs,
            line,
        })
    }

    fn at_reqs(&self) -> bool {
        self.at("uses") || self.at("without") || self.at("limit")
    }

    /// Результат и требования после параметров (`sig_tail` и `fn_tail`), оба необязательны.
    /// После открытого типа — того, что кончается типом функции без скобок, — требований нет:
    /// они уже отнесены к той функции.
    fn sig_tail(&mut self) -> Res<(Option<Type>, Option<Reqs>)> {
        if self.eat("->") {
            self.deeper()?;
            let (ret, open) = self.ty_open()?;
            self.shallower();
            if open || !self.at_reqs() {
                return Ok((Some(ret), None));
            }
            return Ok((Some(ret), Some(self.reqs()?)));
        }
        if self.at_reqs() {
            return Ok((None, Some(self.reqs()?)));
        }
        Ok((None, None))
    }

    fn generics(&mut self) -> Res<Vec<GenericParam>> {
        self.expect("[")?;
        let mut list = Vec::new();
        list.push(self.generic_param()?);
        while self.eat(",") {
            if self.at("]") {
                break;
            }
            list.push(self.generic_param()?);
        }
        self.expect("]")?;
        Ok(list)
    }

    fn generic_param(&mut self) -> Res<GenericParam> {
        if self.eat("uses") {
            let name = self.name()?;
            let without = if self.eat("without") {
                Some(self.req_set()?)
            } else {
                None
            };
            return Ok(GenericParam::Reqs(name, without));
        }
        if self.eat("life") {
            return Ok(GenericParam::Life(self.name()?));
        }
        let name = self.name()?;
        let bound = if self.eat(":") {
            self.bound()?
        } else {
            Vec::new()
        };
        Ok(GenericParam::Type(name, bound))
    }

    fn bound(&mut self) -> Res<Vec<PathType>> {
        let mut list = Vec::new();
        list.push(self.path_type()?);
        while self.eat("and") {
            list.push(self.path_type()?);
        }
        Ok(list)
    }

    /// Параметры до закрывающей скобки; сама скобка остаётся.
    fn params(&mut self) -> Res<Vec<Param>> {
        let mut list = Vec::new();
        if self.at(")") {
            return Ok(list);
        }
        list.push(self.param()?);
        while self.eat(",") {
            if self.at(")") {
                break;
            }
            list.push(self.param()?);
        }
        Ok(list)
    }

    fn param(&mut self) -> Res<Param> {
        let line = self.line();
        let mode = self.mode();
        if self.eat("self") {
            return Ok(Param {
                mode,
                kind: ParamKind::SelfValue,
                line,
            });
        }
        let name = self.name()?;
        let ty = if self.eat(":") {
            Some(self.ty()?)
        } else {
            None
        };
        Ok(Param {
            mode,
            kind: ParamKind::Named(name, ty),
            line,
        })
    }

    /// Требования: `uses`, `without`, `limit` — в этом порядке, хотя бы одно.
    fn reqs(&mut self) -> Res<Reqs> {
        if !self.at_reqs() {
            return self.fail("«uses», «without» или «limit»");
        }
        let uses = if self.eat("uses") {
            Some(self.req_set()?)
        } else {
            None
        };
        let without = if self.eat("without") {
            Some(self.req_set()?)
        } else {
            None
        };
        let limit = if self.eat("limit") {
            self.limit_set()?
        } else {
            Vec::new()
        };
        Ok(Reqs {
            uses,
            without,
            limit,
        })
    }

    fn req_set(&mut self) -> Res<Vec<ReqItem>> {
        let mut list = Vec::new();
        if !self.open("(")? {
            list.push(self.req_item()?);
            return Ok(list);
        }
        if !self.at(")") {
            list.push(self.req_item()?);
            while self.eat(",") {
                if self.at(")") {
                    break;
                }
                list.push(self.req_item()?);
            }
        }
        self.expect(")")?;
        Ok(list)
    }

    fn req_item(&mut self) -> Res<ReqItem> {
        if self.eat("trust") {
            return Ok(ReqItem::Trust);
        }
        Ok(ReqItem::Name(self.dotted()?))
    }

    fn limit_set(&mut self) -> Res<Vec<LimitItem>> {
        let mut list = Vec::new();
        if !self.open("(")? {
            list.push(self.limit_item()?);
            return Ok(list);
        }
        list.push(self.limit_item()?);
        while self.eat(",") {
            if self.at(")") {
                break;
            }
            list.push(self.limit_item()?);
        }
        self.expect(")")?;
        Ok(list)
    }

    fn limit_item(&mut self) -> Res<LimitItem> {
        let name = self.name()?;
        let at_most = if self.eat("<=") {
            true
        } else if self.eat(">=") {
            false
        } else {
            return self.fail("«<=» или «>=»");
        };
        self.deeper()?;
        let value = self.arith()?;
        self.shallower();
        Ok(LimitItem {
            name,
            at_most,
            value,
        })
    }

    fn struct_decl(&mut self) -> Res<StructDecl> {
        self.expect("struct")?;
        let name = self.name()?;
        let generics = if self.at("[") {
            self.generics()?
        } else {
            Vec::new()
        };
        self.expect("{")?;
        let mut fields = Vec::new();
        if !self.at("}") {
            fields.push(self.field()?);
            while self.eat_term() {
                if self.at("}") {
                    break;
                }
                fields.push(self.field()?);
            }
        }
        self.expect("}")?;
        let end = self.end_name()?;
        Ok(StructDecl {
            name,
            generics,
            fields,
            end,
        })
    }

    fn field(&mut self) -> Res<Field> {
        let line = self.line();
        let attrs = self.attrs()?;
        let is_pub = self.eat("pub");
        let name = self.name()?;
        self.expect(":")?;
        let ty = self.ty()?;
        let default = if self.eat("=") {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(Field {
            attrs,
            is_pub,
            name,
            ty,
            default,
            line,
        })
    }

    fn enum_decl(&mut self) -> Res<EnumDecl> {
        self.expect("enum")?;
        let name = self.name()?;
        let generics = if self.at("[") {
            self.generics()?
        } else {
            Vec::new()
        };
        self.expect("{")?;
        let mut variants = Vec::new();
        if !self.at("}") {
            variants.push(self.variant()?);
            while self.eat_term() {
                if self.at("}") {
                    break;
                }
                variants.push(self.variant()?);
            }
        }
        self.expect("}")?;
        let end = self.end_name()?;
        Ok(EnumDecl {
            name,
            generics,
            variants,
            end,
        })
    }

    fn variant(&mut self) -> Res<Variant> {
        let line = self.line();
        let indirect = self.eat("indirect");
        let name = self.name()?;
        let mut fields = VariantFields::None;
        if self.open("(")? {
            if self.at_name() && self.is(1, ":") {
                let mut list = Vec::new();
                list.push(self.named_field()?);
                while self.eat(",") {
                    if self.at(")") {
                        break;
                    }
                    list.push(self.named_field()?);
                }
                fields = VariantFields::Named(list);
            } else {
                fields = VariantFields::Positional(self.ty()?);
            }
            self.expect(")")?;
        }
        Ok(Variant {
            indirect,
            name,
            fields,
            line,
        })
    }

    fn named_field(&mut self) -> Res<NamedField> {
        let name = self.name()?;
        self.expect(":")?;
        let ty = self.ty()?;
        Ok(NamedField { name, ty })
    }

    fn interface_decl(&mut self) -> Res<InterfaceDecl> {
        self.expect("interface")?;
        let name = self.name()?;
        let generics = if self.at("[") {
            self.generics()?
        } else {
            Vec::new()
        };
        let bound = if self.eat(":") {
            self.bound()?
        } else {
            Vec::new()
        };
        self.expect("{")?;
        let members = self.members()?;
        self.expect("}")?;
        let end = self.end_name()?;
        Ok(InterfaceDecl {
            name,
            generics,
            bound,
            members,
            end,
        })
    }

    fn impl_decl(&mut self) -> Res<ImplDecl> {
        self.expect("impl")?;
        let generics = if self.at("[") {
            self.generics()?
        } else {
            Vec::new()
        };
        let first = self.path_type()?;
        let for_type = if self.eat("for") {
            Some(self.path_type()?)
        } else {
            None
        };
        self.expect("{")?;
        let members = self.members()?;
        self.expect("}")?;
        let mut end = None;
        if self.eat("end") {
            self.expect("impl")?;
            let first = self.dotted()?;
            let for_type = if self.eat("for") {
                Some(self.dotted()?)
            } else {
                None
            };
            end = Some(EndImpl { first, for_type });
        }
        Ok(ImplDecl {
            generics,
            first,
            for_type,
            members,
            end,
        })
    }

    /// Члены до закрывающей фигурной скобки; сама скобка остаётся.
    fn members(&mut self) -> Res<Vec<Member>> {
        let mut list = Vec::new();
        if self.at("}") {
            return Ok(list);
        }
        list.push(self.member()?);
        while self.eat_term() {
            if self.at("}") {
                break;
            }
            list.push(self.member()?);
        }
        Ok(list)
    }

    fn member(&mut self) -> Res<Member> {
        let line = self.line();
        let attrs = self.attrs()?;
        let is_pub = self.eat("pub");
        let kind = if self.at("let") || self.at("var") || self.at("const") {
            MemberKind::Binding(self.binding()?)
        } else if self.eat("extern") {
            let abi = self.string()?;
            MemberKind::Func(self.func_decl(Some(abi))?)
        } else if self.at("func") {
            let sig = self.func_sig()?;
            if self.at("{") {
                let body = self.block()?;
                let end = self.end_name()?;
                MemberKind::Func(FuncDecl {
                    abi: None,
                    sig,
                    body,
                    end,
                })
            } else {
                MemberKind::Sig(sig)
            }
        } else {
            return self.fail("функция или связывание");
        };
        Ok(Member {
            attrs,
            is_pub,
            kind,
            line,
        })
    }

    fn type_decl(&mut self) -> Res<TypeAlias> {
        self.expect("type")?;
        let name = self.name()?;
        let generics = if self.at("[") {
            self.generics()?
        } else {
            Vec::new()
        };
        self.expect("=")?;
        let ty = self.ty()?;
        Ok(TypeAlias { name, generics, ty })
    }

    fn test_decl(&mut self) -> Res<TestDecl> {
        self.expect("test")?;
        let name = self.string()?;
        let body = self.block()?;
        let has_end = self.end_kw("test")?;
        Ok(TestDecl {
            name,
            body,
            has_end,
        })
    }

    /// `extern "C" trust "обоснование" { … }`; слово `extern` и первая строка уже прочитаны.
    fn extern_block(&mut self, abi: StrLit) -> Res<ExternBlock> {
        self.expect("trust")?;
        let reason = self.string()?;
        self.expect("{")?;
        let members = self.members()?;
        self.expect("}")?;
        let has_end = self.end_kw("extern")?;
        Ok(ExternBlock {
            abi,
            reason,
            members,
            has_end,
        })
    }

    // ───────── типы ─────────

    fn ty(&mut self) -> Res<Type> {
        let (ty, _) = self.ty_open()?;
        Ok(ty)
    }

    /// Тип и признак «открытый»: кончается типом функции, не взятым в скобки.
    fn ty_open(&mut self) -> Res<(Type, bool)> {
        let line = self.line();
        if !self.open("(")? {
            return self.prefixed_type();
        }
        if self.eat(")") {
            return Ok((
                Type {
                    kind: TypeKind::Unit,
                    line,
                },
                false,
            ));
        }
        let first = self.ty()?;
        if self.eat(")") {
            return Ok((first, false));
        }
        self.expect(",")?;
        let mut items = Vec::new();
        items.push(first);
        items.push(self.ty()?);
        while self.eat(",") {
            if self.at(")") {
                break;
            }
            items.push(self.ty()?);
        }
        self.expect(")")?;
        Ok((
            Type {
                kind: TypeKind::Tuple(items),
                line,
            },
            false,
        ))
    }

    /// `simple_type` и `open_type`: имя типа, `any`, тип функции и они же после `ref` и `ptr`.
    fn prefixed_type(&mut self) -> Res<(Type, bool)> {
        let line = self.line();
        if self.at("ref") || (self.at("mut") && self.is(1, "ref")) {
            let mutable = self.eat("mut");
            self.pos += 1;
            self.deeper()?;
            let origin = self.origin()?;
            let (inner, open) = self.prefixed_type()?;
            self.shallower();
            let kind = TypeKind::Ref(mutable, origin, Box::new(inner));
            return Ok((Type { kind, line }, open));
        }
        if self.at("ptr") || (self.at("mut") && self.is(1, "ptr")) {
            let mutable = self.eat("mut");
            self.pos += 1;
            self.deeper()?;
            let (inner, open) = self.prefixed_type()?;
            self.shallower();
            let kind = TypeKind::Ptr(mutable, Box::new(inner));
            return Ok((Type { kind, line }, open));
        }
        if self.at("any") && !self.is(1, "func") {
            self.pos += 1;
            let path = if self.at_name() {
                Some(self.path_type()?)
            } else {
                None
            };
            let kind = TypeKind::Any(path);
            return Ok((Type { kind, line }, false));
        }
        if self.at("func")
            || self.at("mut")
            || self.at("take")
            || self.at("any")
            || self.at("extern")
        {
            let kind = TypeKind::Func(Box::new(self.fn_type()?));
            return Ok((Type { kind, line }, true));
        }
        if self.at_name() {
            let kind = TypeKind::Path(self.path_type()?);
            return Ok((Type { kind, line }, false));
        }
        self.fail("тип")
    }

    /// Источник ссылки в скобках сразу после `ref`.
    fn origin(&mut self) -> Res<Option<Origin>> {
        if !self.open("(")? {
            return Ok(None);
        }
        let origin = if self.eat("self") {
            Origin::SelfValue
        } else {
            Origin::Name(self.name()?)
        };
        self.expect(")")?;
        Ok(Some(origin))
    }

    fn path_type(&mut self) -> Res<PathType> {
        let line = self.line();
        let names = self.dotted()?;
        let args = if self.at("[") {
            self.generic_args()?
        } else {
            Vec::new()
        };
        Ok(PathType { names, args, line })
    }

    fn generic_args(&mut self) -> Res<Vec<GenericArg>> {
        self.expect("[")?;
        let mut list = Vec::new();
        list.push(self.generic_arg()?);
        while self.eat(",") {
            if self.at("]") {
                break;
            }
            list.push(self.generic_arg()?);
        }
        self.expect("]")?;
        Ok(list)
    }

    fn generic_arg(&mut self) -> Res<GenericArg> {
        if self.kind_is(0, TokenKind::Int) {
            return Ok(GenericArg::Int(self.take()));
        }
        if self.eat("uses") {
            return Ok(GenericArg::Uses(self.req_set()?));
        }
        if self.eat("life") {
            if self.eat("self") {
                return Ok(GenericArg::Life(LifeSource::SelfValue));
            }
            let mut names = Vec::new();
            if self.open("(")? {
                names.push(self.name()?);
                while self.eat(",") {
                    names.push(self.name()?);
                }
                self.expect(")")?;
            } else {
                names.push(self.name()?);
            }
            return Ok(GenericArg::Life(LifeSource::Names(names)));
        }
        Ok(GenericArg::Type(self.ty()?))
    }

    fn fn_type(&mut self) -> Res<FuncType> {
        let kind = if self.eat("mut") {
            Some(FuncKind::Mut)
        } else if self.eat("take") {
            Some(FuncKind::Take)
        } else if self.eat("any") {
            Some(FuncKind::Any)
        } else if self.eat("extern") {
            Some(FuncKind::Extern(self.string()?))
        } else {
            None
        };
        self.expect("func")?;
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.at(")") {
            params.push(self.fn_param()?);
            while self.eat(",") {
                if self.at(")") {
                    break;
                }
                params.push(self.fn_param()?);
            }
        }
        self.expect(")")?;
        let (ret, reqs) = self.sig_tail()?;
        Ok(FuncType {
            kind,
            params,
            ret,
            reqs,
        })
    }

    /// Параметр в типе функции: тип или `режим? имя: тип`; вместо имени может стоять `_`.
    fn fn_param(&mut self) -> Res<FuncTypeParam> {
        let has_mode = self.at("mut") || self.at("take");
        let k = if has_mode { 1 } else { 0 };
        let named = (self.kind_is(k, TokenKind::Name) || self.is(k, "_")) && self.is(k + 1, ":");
        if !named {
            let ty = self.ty()?;
            return Ok(FuncTypeParam {
                mode: None,
                name: None,
                ty,
            });
        }
        let mode = self.mode();
        let name = self.take();
        self.pos += 1;
        let ty = self.ty()?;
        Ok(FuncTypeParam {
            mode,
            name: Some(name),
            ty,
        })
    }

    // ───────── инструкции ─────────

    fn block(&mut self) -> Res<Block> {
        let line = self.line();
        self.expect("{")?;
        let stmts = if self.at("}") {
            Vec::new()
        } else {
            self.stmt_list()?
        };
        self.expect("}")?;
        Ok(Block { stmts, line })
    }

    /// `stmt (TERM stmt)* TERM?` — до закрывающей скобки или конца текста.
    fn stmt_list(&mut self) -> Res<Vec<Stmt>> {
        let mut list = Vec::new();
        list.push(self.stmt()?);
        while self.eat_term() {
            if self.at("}") || self.at_end() {
                break;
            }
            list.push(self.stmt()?);
        }
        Ok(list)
    }

    fn stmt(&mut self) -> Res<Stmt> {
        if self.at("let") || self.at("var") || self.at("const") {
            let line = self.line();
            let kind = StmtKind::Binding(self.binding()?);
            return Ok(Stmt { kind, line });
        }
        self.plain_stmt()
    }

    fn plain_stmt(&mut self) -> Res<Stmt> {
        let line = self.line();
        if self.eat("return") {
            let value = if self.kind_is(0, TokenKind::Term) || self.at("}") || self.at_end() {
                None
            } else {
                Some(self.expr()?)
            };
            let kind = StmtKind::Return(value);
            return Ok(Stmt { kind, line });
        }
        if self.at("break") || self.at("continue") {
            let is_break = self.at("break");
            self.pos += 1;
            let label = if self.at_name() {
                Some(self.take())
            } else {
                None
            };
            let kind = if is_break {
                StmtKind::Break(label)
            } else {
                StmtKind::Continue(label)
            };
            return Ok(Stmt { kind, line });
        }
        if self.eat("assert") {
            let cond = self.expr()?;
            let message = if self.eat(",") {
                Some(self.expr()?)
            } else {
                None
            };
            let kind = StmtKind::Assert(cond, message);
            return Ok(Stmt { kind, line });
        }
        if self.eat("discard") {
            self.expect("self")?;
            let kind = StmtKind::DiscardSelf;
            return Ok(Stmt { kind, line });
        }
        let mut label = None;
        if self.at_name()
            && self.is(1, ":")
            && (self.is(2, "for") || self.is(2, "while") || self.is(2, "loop"))
        {
            label = Some(self.take());
            self.pos += 1;
        }
        if self.eat("for") {
            let target = if self.eat("_") {
                ForTarget::Wildcard
            } else if self.at("(") {
                ForTarget::Tuple(self.tuple_pattern()?)
            } else {
                ForTarget::Name(self.name()?)
            };
            self.expect("in")?;
            let mode = self.mode();
            self.deeper()?;
            let iter = self.expr()?;
            self.shallower();
            let body = self.block()?;
            let end = self.end_loop()?;
            let kind = StmtKind::For(ForStmt {
                label,
                target,
                mode,
                iter,
                body,
                end,
            });
            return Ok(Stmt { kind, line });
        }
        if self.eat("while") {
            self.deeper()?;
            let cond = self.expr()?;
            self.shallower();
            let body = self.block()?;
            let end = self.end_loop()?;
            let kind = StmtKind::While(WhileStmt {
                label,
                cond,
                body,
                end,
            });
            return Ok(Stmt { kind, line });
        }
        if self.eat("loop") {
            let body = self.block()?;
            let end = self.end_loop()?;
            let kind = StmtKind::Loop(LoopStmt { label, body, end });
            return Ok(Stmt { kind, line });
        }
        let value = self.expr()?;
        let op = match self.assign_op() {
            Some(op) => op,
            None => {
                let kind = StmtKind::Expr(value);
                return Ok(Stmt { kind, line });
            }
        };
        if !is_postfix_form(&value) {
            return Err(ParseError {
                line: self.line(),
                message: String::from(
                    "слева от знака присваивания может стоять имя, поле, элемент или вызов, но не операция",
                ),
            });
        }
        self.pos += 1;
        let right = self.expr()?;
        let kind = StmtKind::Assign(op, value, right);
        Ok(Stmt { kind, line })
    }

    /// Знак присваивания под указателем: `None` — его нет; `Some(None)` — простое `=`.
    fn assign_op(&self) -> Option<Option<BinOp>> {
        const OPS: &[(&str, Option<BinOp>)] = &[
            ("=", None),
            ("+=", Some(BinOp::Add)),
            ("-=", Some(BinOp::Sub)),
            ("*=", Some(BinOp::Mul)),
            ("/=", Some(BinOp::Div)),
            ("%=", Some(BinOp::Rem)),
            ("&=", Some(BinOp::BitAnd)),
            ("|=", Some(BinOp::BitOr)),
            ("^=", Some(BinOp::BitXor)),
            ("<<=", Some(BinOp::Shl)),
            (">>=", Some(BinOp::Shr)),
        ];
        for &(text, op) in OPS {
            if self.at(text) {
                return Some(op);
            }
        }
        None
    }

    /// Необязательная метка после цикла: `end for`, `end while`, `end loop` или `end имя`.
    fn end_loop(&mut self) -> Res<Option<EndLoop>> {
        if !self.eat("end") {
            return Ok(None);
        }
        if self.at("for") || self.at("while") || self.at("loop") {
            return Ok(Some(EndLoop::Keyword(self.take())));
        }
        Ok(Some(EndLoop::Label(self.label_word()?)))
    }

    // ───────── выражения ─────────

    /// `and` и `or` не смешиваются: цепочка состоит из одного и того же слова.
    fn expr(&mut self) -> Res<Expr> {
        let first = self.not_expr()?;
        let mut rest = Vec::new();
        if self.at("or") {
            while self.eat("or") {
                rest.push((BinOp::Or, self.not_expr()?));
            }
        } else if self.at("and") {
            while self.eat("and") {
                rest.push((BinOp::And, self.not_expr()?));
            }
        }
        Ok(chain(first, rest))
    }

    fn not_expr(&mut self) -> Res<Expr> {
        let line = self.line();
        if self.eat("not") {
            self.deeper()?;
            let inner = self.not_expr()?;
            self.shallower();
            let kind = ExprKind::Unary(UnOp::Not, Box::new(inner));
            return Ok(Expr { kind, line });
        }
        self.comparison()
    }

    /// Сравнения не выстраиваются в цепочку: после одного сравнения второго нет.
    fn comparison(&mut self) -> Res<Expr> {
        const OPS: &[(&str, BinOp)] = &[
            ("==", BinOp::Eq),
            ("!=", BinOp::Ne),
            ("<", BinOp::Lt),
            ("<=", BinOp::Le),
            (">", BinOp::Gt),
            (">=", BinOp::Ge),
        ];
        let left = self.operand()?;
        for &(text, op) in OPS {
            if self.eat(text) {
                let right = self.operand()?;
                return Ok(chain(left, vec![(op, right)]));
            }
        }
        if self.eat("is") {
            let line = left.line;
            let pattern = self.pattern()?;
            let kind = ExprKind::Is(Box::new(left), pattern);
            return Ok(Expr { kind, line });
        }
        Ok(left)
    }

    /// Одно семейство операций: арифметика (и диапазон из двух арифметических частей),
    /// `&`, `|`, `^` или один сдвиг. Семейства между собой смешиваются только через скобки.
    fn operand(&mut self) -> Res<Expr> {
        let first = self.unary()?;
        if self.at("&") {
            return self.bit_chain(first, "&", BinOp::BitAnd);
        }
        if self.at("|") {
            return self.bit_chain(first, "|", BinOp::BitOr);
        }
        if self.at("^") {
            return self.bit_chain(first, "^", BinOp::BitXor);
        }
        if self.at("<<") || self.at(">>") {
            let op = if self.at("<<") {
                BinOp::Shl
            } else {
                BinOp::Shr
            };
            self.pos += 1;
            let right = self.unary()?;
            return Ok(chain(first, vec![(op, right)]));
        }
        let left = self.arith_from(first)?;
        if self.at("..<") || self.at("..=") {
            let op = if self.at("..<") {
                BinOp::RangeExcl
            } else {
                BinOp::RangeIncl
            };
            self.pos += 1;
            let right = self.arith()?;
            return Ok(chain(left, vec![(op, right)]));
        }
        Ok(left)
    }

    fn bit_chain(&mut self, first: Expr, sign: &str, op: BinOp) -> Res<Expr> {
        let mut rest = Vec::new();
        while self.eat(sign) {
            rest.push((op, self.unary()?));
        }
        Ok(chain(first, rest))
    }

    fn arith(&mut self) -> Res<Expr> {
        let first = self.unary()?;
        self.arith_from(first)
    }

    /// Сложение и вычитание; `first` — уже разобранная первая часть.
    fn arith_from(&mut self, first: Expr) -> Res<Expr> {
        let head = self.term_from(first)?;
        let mut rest = Vec::new();
        loop {
            let op = if self.at("+") {
                BinOp::Add
            } else if self.at("-") {
                BinOp::Sub
            } else {
                break;
            };
            self.pos += 1;
            let next = self.unary()?;
            rest.push((op, self.term_from(next)?));
        }
        Ok(chain(head, rest))
    }

    /// Умножение, деление и остаток; `first` — уже разобранная первая часть.
    fn term_from(&mut self, first: Expr) -> Res<Expr> {
        let mut rest = Vec::new();
        loop {
            let op = if self.at("*") {
                BinOp::Mul
            } else if self.at("/") {
                BinOp::Div
            } else if self.at("%") {
                BinOp::Rem
            } else {
                break;
            };
            self.pos += 1;
            rest.push((op, self.unary()?));
        }
        Ok(chain(first, rest))
    }

    /// Операнд и необязательный `catch` после него.
    fn unary(&mut self) -> Res<Expr> {
        let value = self.prefix()?;
        if !self.eat("catch") {
            return Ok(value);
        }
        let line = value.line;
        let name = if self.at_name() {
            Some(self.take())
        } else {
            None
        };
        let body = self.block()?;
        let has_end = self.end_kw("catch")?;
        let kind = ExprKind::Catch(Box::new(CatchExpr {
            value,
            name,
            body,
            has_end,
        }));
        Ok(Expr { kind, line })
    }

    fn prefix(&mut self) -> Res<Expr> {
        let line = self.line();
        let op = if self.at("-") {
            UnOp::Neg
        } else if self.at("~") {
            UnOp::BitNot
        } else if self.at("try") {
            UnOp::Try
        } else if self.at("await") {
            UnOp::Await
        } else {
            return self.postfix();
        };
        self.pos += 1;
        self.deeper()?;
        let inner = self.prefix()?;
        self.shallower();
        let kind = ExprKind::Unary(op, Box::new(inner));
        Ok(Expr { kind, line })
    }

    /// Основа и всё, что к ней приписано: `.имя`, вызовы и индексы — одним списком.
    fn postfix(&mut self) -> Res<Expr> {
        let base = self.primary()?;
        let mut ops = Vec::new();
        loop {
            if self.eat(".") {
                ops.push(PostfixOp::Field(self.name()?));
            } else if self.open("(")? {
                let args = self.args()?;
                self.expect(")")?;
                ops.push(PostfixOp::Call(args));
            } else if self.open("[")? {
                let mut args = Vec::new();
                args.push(self.index_arg()?);
                while self.eat(",") {
                    if self.at("]") {
                        break;
                    }
                    args.push(self.index_arg()?);
                }
                self.expect("]")?;
                ops.push(PostfixOp::Index(args));
            } else {
                break;
            }
        }
        if ops.is_empty() {
            return Ok(base);
        }
        let line = base.line;
        let kind = ExprKind::Postfix(Box::new(base), ops);
        Ok(Expr { kind, line })
    }

    /// Аргументы вызова до закрывающей скобки; сама скобка остаётся.
    fn args(&mut self) -> Res<Vec<Arg>> {
        let mut list = Vec::new();
        if self.at(")") {
            return Ok(list);
        }
        list.push(self.arg()?);
        while self.eat(",") {
            if self.at(")") {
                break;
            }
            list.push(self.arg()?);
        }
        Ok(list)
    }

    fn arg(&mut self) -> Res<Arg> {
        if self.eat("mut") {
            return Ok(Arg::Mut(self.expr()?));
        }
        if self.at_name() && self.is(1, ":") {
            let name = self.take();
            self.pos += 1;
            return Ok(Arg::Named(name, self.expr()?));
        }
        Ok(Arg::Plain(self.expr()?))
    }

    fn index_arg(&mut self) -> Res<IndexArg> {
        if self.eat("any") {
            if self.at_name() {
                return Ok(IndexArg::Any(Some(self.path_type()?)));
            }
            return Ok(IndexArg::Any(None));
        }
        if self.eat("uses") {
            return Ok(IndexArg::Uses(self.req_set()?));
        }
        Ok(IndexArg::Expr(self.expr()?))
    }

    fn primary(&mut self) -> Res<Expr> {
        let line = self.line();
        if self.at_end() {
            return self.fail("выражение");
        }
        let kind = match self.toks[self.pos].kind {
            TokenKind::Name => ExprKind::Name(self.take()),
            TokenKind::Int => ExprKind::Int(self.take()),
            TokenKind::Float => ExprKind::Float(self.take()),
            TokenKind::Char => ExprKind::Char(self.take()),
            TokenKind::Str => ExprKind::Str(self.string()?),
            _ => {
                if self.eat("self") {
                    ExprKind::SelfValue
                } else if self.eat("true") {
                    ExprKind::Bool(true)
                } else if self.eat("false") {
                    ExprKind::Bool(false)
                } else if self.open("(")? {
                    self.after_paren()?
                } else if self.open("[")? {
                    let mut items = Vec::new();
                    if !self.at("]") {
                        items.push(self.expr()?);
                        while self.eat(",") {
                            if self.at("]") {
                                break;
                            }
                            items.push(self.expr()?);
                        }
                    }
                    self.expect("]")?;
                    ExprKind::List(items)
                } else if self.at("if") {
                    let mut whole = self.if_chain()?;
                    whole.has_end = self.end_kw("if")?;
                    ExprKind::If(Box::new(whole))
                } else if self.at("match") {
                    ExprKind::Match(Box::new(self.match_expr()?))
                } else if self.at("func") {
                    ExprKind::Closure(Box::new(self.closure()?))
                } else if self.at("trust") {
                    ExprKind::Trust(Box::new(self.trust_expr()?))
                } else if self.at("provide") {
                    ExprKind::Provide(Box::new(self.provide_expr()?))
                } else if self.at("target") {
                    ExprKind::Target(Box::new(self.target_expr()?))
                } else if self.at_marker() {
                    ExprKind::LevelBlock(Box::new(self.level_block()?))
                } else {
                    return self.fail("выражение");
                }
            }
        };
        Ok(Expr { kind, line })
    }

    /// После открывающей круглой скобки: `()`, выражение в скобках или кортеж из двух и больше.
    fn after_paren(&mut self) -> Res<ExprKind> {
        if self.eat(")") {
            return Ok(ExprKind::Unit);
        }
        let first = self.expr()?;
        if self.eat(")") {
            return Ok(ExprKind::Paren(Box::new(first)));
        }
        self.expect(",")?;
        let mut items = Vec::new();
        items.push(first);
        items.push(self.expr()?);
        while self.eat(",") {
            if self.at(")") {
                break;
            }
            items.push(self.expr()?);
        }
        self.expect(")")?;
        Ok(ExprKind::Tuple(items))
    }

    /// `if условие блок`, дальше сколько угодно `else if …` и необязательный `else блок`.
    /// Метку `end if` читает вызывающий.
    fn if_chain(&mut self) -> Res<IfExpr> {
        let mut branches = Vec::new();
        let mut otherwise = None;
        loop {
            self.expect("if")?;
            self.deeper()?;
            let cond = self.expr()?;
            self.shallower();
            let then = self.block()?;
            branches.push(IfBranch { cond, then });
            if !self.eat("else") {
                break;
            }
            if !self.at("if") {
                otherwise = Some(self.block()?);
                break;
            }
        }
        Ok(IfExpr {
            branches,
            otherwise,
            has_end: false,
        })
    }

    fn match_expr(&mut self) -> Res<MatchExpr> {
        self.expect("match")?;
        let mode = self.mode();
        self.deeper()?;
        let value = self.expr()?;
        self.shallower();
        self.expect("{")?;
        let mut items = Vec::new();
        while self.at("case") || self.at("else") {
            items.push(self.match_item()?);
            self.eat_term();
        }
        self.expect("}")?;
        let has_end = self.end_kw("match")?;
        Ok(MatchExpr {
            mode,
            value,
            items,
            has_end,
        })
    }

    fn match_item(&mut self) -> Res<MatchItem> {
        if self.eat("else") {
            let body = self.block()?;
            let has_end = self.end_kw("else")?;
            return Ok(MatchItem::Else(body, has_end));
        }
        self.expect("case")?;
        let mut patterns = Vec::new();
        patterns.push(self.pattern()?);
        while self.eat(",") {
            patterns.push(self.pattern()?);
        }
        let mut guard = None;
        if self.eat("if") {
            self.deeper()?;
            guard = Some(self.expr()?);
            self.shallower();
        }
        let body = self.block()?;
        let has_end = self.end_kw("case")?;
        Ok(MatchItem::Case(MatchCase {
            patterns,
            guard,
            body,
            has_end,
        }))
    }

    fn closure(&mut self) -> Res<Closure> {
        self.expect("func")?;
        self.expect("(")?;
        let params = self.params()?;
        self.expect(")")?;
        let (ret, reqs) = self.sig_tail()?;
        let body = self.block()?;
        let has_end = self.end_kw("func")?;
        Ok(Closure {
            params,
            ret,
            reqs,
            body,
            has_end,
        })
    }

    fn trust_expr(&mut self) -> Res<TrustExpr> {
        self.expect("trust")?;
        let reason = self.string()?;
        let body = self.block()?;
        let has_end = self.end_kw("trust")?;
        Ok(TrustExpr {
            reason,
            body,
            has_end,
        })
    }

    fn provide_expr(&mut self) -> Res<ProvideExpr> {
        self.expect("provide")?;
        let mut items = Vec::new();
        if self.open("(")? {
            items.push(self.provide_item()?);
            while self.eat(",") {
                if self.at(")") {
                    break;
                }
                items.push(self.provide_item()?);
            }
            self.expect(")")?;
        } else {
            items.push(self.provide_item()?);
        }
        let body = self.block()?;
        let has_end = self.end_kw("provide")?;
        Ok(ProvideExpr {
            items,
            body,
            has_end,
        })
    }

    fn provide_item(&mut self) -> Res<ProvideItem> {
        let name = self.name()?;
        self.expect("=")?;
        self.deeper()?;
        let value = self.expr()?;
        self.shallower();
        Ok(ProvideItem { name, value })
    }

    fn target_expr(&mut self) -> Res<TargetExpr> {
        self.expect("target")?;
        self.expect("{")?;
        let mut items = Vec::new();
        while self.at("case") || self.at("else") {
            if self.eat("else") {
                let body = self.block()?;
                let has_end = self.end_kw("else")?;
                items.push(TargetItem::Else(body, has_end));
            } else {
                self.pos += 1;
                let name = self.dotted()?;
                let body = self.block()?;
                let has_end = self.end_kw("case")?;
                items.push(TargetItem::Case(name, body, has_end));
            }
            self.eat_term();
        }
        self.expect("}")?;
        let has_end = self.end_kw("target")?;
        Ok(TargetExpr { items, has_end })
    }

    fn level_block(&mut self) -> Res<LevelBlock> {
        let marker = self.take();
        let body = self.block()?;
        let mut end = None;
        if self.eat("end") {
            if !self.at_marker() {
                return self.fail("пометка уровня или строгости");
            }
            end = Some(self.take());
        }
        Ok(LevelBlock { marker, body, end })
    }

    // ───────── образцы ─────────

    fn pattern(&mut self) -> Res<Pattern> {
        let line = self.line();
        if self.at("(") {
            return self.tuple_pattern();
        }
        if self.at_end() {
            return self.fail("образец");
        }
        let kind = match self.toks[self.pos].kind {
            TokenKind::Int => PatternKind::Int(false, self.take()),
            TokenKind::Float => PatternKind::Float(false, self.take()),
            TokenKind::Char => PatternKind::Char(self.take()),
            TokenKind::Str => PatternKind::Str(self.string()?),
            TokenKind::Name => {
                let path = self.dotted()?;
                if self.open("(")? {
                    let mut args = Vec::new();
                    if !self.at(")") {
                        args.push(self.pattern_arg()?);
                        while self.eat(",") {
                            if self.at(")") {
                                break;
                            }
                            args.push(self.pattern_arg()?);
                        }
                    }
                    self.expect(")")?;
                    PatternKind::Ctor(path, args)
                } else {
                    PatternKind::Path(path)
                }
            }
            _ => {
                if self.eat("_") {
                    PatternKind::Wildcard
                } else if self.eat("true") {
                    PatternKind::Bool(true)
                } else if self.eat("false") {
                    PatternKind::Bool(false)
                } else if self.eat("-") {
                    if self.kind_is(0, TokenKind::Int) {
                        PatternKind::Int(true, self.take())
                    } else if self.kind_is(0, TokenKind::Float) {
                        PatternKind::Float(true, self.take())
                    } else {
                        return self.fail("число");
                    }
                } else {
                    return self.fail("образец");
                }
            }
        };
        Ok(Pattern { kind, line })
    }

    fn pattern_arg(&mut self) -> Res<PatternArg> {
        if self.eat("..") {
            return Ok(PatternArg::Rest);
        }
        if self.at_name() && self.is(1, ":") {
            let name = self.take();
            self.pos += 1;
            return Ok(PatternArg::Field(name, self.pattern()?));
        }
        Ok(PatternArg::Pattern(self.pattern()?))
    }

    /// Образец-кортеж: в скобках не меньше двух образцов.
    fn tuple_pattern(&mut self) -> Res<Pattern> {
        let line = self.line();
        self.expect("(")?;
        let mut items = Vec::new();
        items.push(self.pattern()?);
        self.expect(",")?;
        items.push(self.pattern()?);
        while self.eat(",") {
            if self.at(")") {
                break;
            }
            items.push(self.pattern()?);
        }
        self.expect(")")?;
        let kind = PatternKind::Tuple(items);
        Ok(Pattern { kind, line })
    }
}
