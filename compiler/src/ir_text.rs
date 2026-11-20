//! Текстовая форма IR: запись и чтение.
//!
//! `print` даёт одну каноническую запись модуля, `parse` читает её обратно без потерь:
//! `parse(&print(m)) == Ok(m)`, а для канонического текста `print(&parse(t)?) == t`.
//! Чтение допускает любые пробелы и переводы строк и комментарии от `;` до конца строки.
//! Имя пишется в обратных кавычках, если оно не из букв, цифр и `_`, начинается с цифры или
//! совпадает со словом формы или с именем скаляра: `` `copy` ``. Описание формы —
//! `spec/ir-core.md`, раздел 3 «Текстовая форма».

use crate::ir::*;
use crate::lexer::{ident_cont, ident_start};

/// Слова текстовой формы. Имена с таким написанием пишутся в обратных кавычках.
const WORDS: &[&str] = &[
    "ir", "struct", "enum", "copy", "func", "test", "uses", "mut", "take", "local", "drop", "call",
    "if", "else", "switch", "case", "loop", "break", "continue", "return", "crash", "move",
    "borrow", "const", "make", "tuple", "array", "conv", "held", "true", "false", "Array",
];

fn reserved(s: &str) -> bool {
    WORDS.contains(&s) || Scalar::from_name(s).is_some()
}

fn plain(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if ident_start(c) => {}
        _ => return false,
    }
    chars.all(ident_cont) && !reserved(s)
}

// ───────── запись ─────────

/// Каноническая запись модуля.
pub fn print(m: &Module) -> String {
    let mut out = format!("ir {}\n", m.version);
    for d in &m.decls {
        out.push('\n');
        out.push_str(&decl(d));
        out.push('\n');
    }
    out
}

fn name(s: &str) -> String {
    if plain(s) {
        s.to_string()
    } else {
        format!("`{}`", s)
    }
}

/// Имя функции может быть составным: `Point.norm`.
fn func_name(s: &str) -> String {
    s.split('.').map(name).collect::<Vec<_>>().join(".")
}

fn join<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    items.iter().map(f).collect::<Vec<_>>().join(", ")
}

/// Содержимое фигурных скобок в одну строку: `{ a, b }` или `{ }`.
fn braces(inner: String) -> String {
    if inner.is_empty() {
        "{ }".to_string()
    } else {
        format!("{{ {} }}", inner)
    }
}

fn field(f: &Field) -> String {
    format!("{}: {}", name(&f.name), ty(&f.ty))
}

fn decl(d: &Decl) -> String {
    match d {
        Decl::Struct(s) => format!(
            "{}struct {} {}",
            if s.copy { "copy " } else { "" },
            name(&s.name),
            braces(join(&s.fields, field))
        ),
        Decl::Enum(e) => format!(
            "{}enum {} {}",
            if e.copy { "copy " } else { "" },
            name(&e.name),
            braces(join(&e.variants, variant))
        ),
        Decl::Func(f) => {
            let mut head = format!("func {}({})", func_name(&f.name), join(&f.params, param));
            if f.result != Type::Unit {
                head.push_str(&format!(" -> {}", ty(&f.result)));
            }
            if !f.uses.is_empty() {
                head.push_str(&format!(" uses ({})", join(&f.uses, |u| name(u))));
            }
            format!("{} {}", head, block(&f.body, 0))
        }
        Decl::Test(t) => format!("test {} {}", string(&t.name), block(&t.body, 0)),
    }
}

fn variant(v: &Variant) -> String {
    match &v.payload {
        Payload::None => name(&v.name),
        Payload::One(t) => format!("{}({})", name(&v.name), ty(t)),
        Payload::Fields(fs) => format!("{}({})", name(&v.name), join(fs, field)),
    }
}

fn param(p: &Param) -> String {
    let mode = match p.mode {
        Mode::Read => "",
        Mode::Mut => " mut",
        Mode::Take => " take",
    };
    format!("{}: {}{}", name(&p.name), ty(&p.ty), mode)
}

/// Тип в текстовой форме — для сообщений.
pub(crate) fn ty(t: &Type) -> String {
    match t {
        Type::Scalar(s) => s.name().to_string(),
        Type::Unit => "()".to_string(),
        Type::Tuple(items) => format!("({})", join(items, ty)),
        Type::Array(elem, n) => format!("Array[{}, {}]", ty(elem), n),
        Type::Named(n) => name(n),
    }
}

fn indent(level: usize) -> String {
    "  ".repeat(level)
}

fn block(b: &Block, level: usize) -> String {
    if b.stmts.is_empty() {
        return "{ }".to_string();
    }
    let mut out = String::from("{\n");
    for s in &b.stmts {
        out.push_str(&indent(level + 1));
        out.push_str(&stmt(s, level + 1));
        out.push('\n');
    }
    out.push_str(&indent(level));
    out.push('}');
    out
}

fn after(s: &Option<State>) -> String {
    match s {
        Some(st) => format!(" {}", state(st)),
        None => String::new(),
    }
}

fn state(s: &State) -> String {
    if s.held.is_empty() {
        "[held:]".to_string()
    } else {
        format!("[held: {}]", join(&s.held, place))
    }
}

fn stmt(s: &Stmt, level: usize) -> String {
    match s {
        Stmt::Local { name: n, ty: t } => format!("local {}: {}", name(n), ty(t)),
        Stmt::Define {
            name: n,
            ty: t,
            value,
        } => format!("{}: {} = {}", name(n), ty(t), rhs(value)),
        Stmt::Assign { place: p, value } => format!("{} = {}", place(p), rhs(value)),
        Stmt::Drop(p) => format!("drop {}", place(p)),
        Stmt::Call { func, args } => format!("call {}({})", func_name(func), join(args, operand)),
        Stmt::If {
            cond,
            then,
            otherwise,
            after: a,
        } => format!(
            "if {} {} else {}{}",
            place(cond),
            block(then, level),
            block(otherwise, level),
            after(a)
        ),
        Stmt::Switch {
            on,
            cases,
            otherwise,
            after: a,
        } => {
            let mut out = format!("switch {} ", place(on));
            if cases.is_empty() && otherwise.is_none() {
                out.push_str("{ }");
            } else {
                out.push_str("{\n");
                for c in cases {
                    out.push_str(&format!(
                        "{}case {} {}\n",
                        indent(level + 1),
                        case_label(&c.label),
                        block(&c.body, level + 1)
                    ));
                }
                if let Some(b) = otherwise {
                    out.push_str(&format!(
                        "{}else {}\n",
                        indent(level + 1),
                        block(b, level + 1)
                    ));
                }
                out.push_str(&indent(level));
                out.push('}');
            }
            out.push_str(&after(a));
            out
        }
        Stmt::Loop {
            label,
            head,
            body,
            after: a,
        } => format!(
            "loop {} {} {}{}",
            name(label),
            state(head),
            block(body, level),
            after(a)
        ),
        Stmt::Break(l) => format!("break {}", name(l)),
        Stmt::Continue(l) => format!("continue {}", name(l)),
        Stmt::Return(None) => "return".to_string(),
        Stmt::Return(Some(v)) => format!("return {}", operand(v)),
        Stmt::Crash(msg) => format!("crash {}", string(msg)),
    }
}

fn case_label(l: &CaseLabel) -> String {
    match l {
        CaseLabel::Variant(v) => name(v),
        CaseLabel::Value(c) => constant(c),
    }
}

fn rhs(r: &Rhs) -> String {
    match r {
        Rhs::Use(v) => operand(v),
        Rhs::Op { op, ty, args } => format!("{}.{}({})", op.name(), ty.name(), join(args, operand)),
        Rhs::Conv { from, to, arg } => {
            format!("conv.{}.{}({})", from.name(), to.name(), operand(arg))
        }
        Rhs::Call { func, args } => format!("call {}({})", func_name(func), join(args, operand)),
        Rhs::Make { ty, variant, args } => {
            let mut out = format!("make {}", name(ty));
            if let Some(v) = variant {
                out.push('.');
                out.push_str(&name(v));
            }
            if let Some(a) = args {
                out.push_str(&format!("({})", join(a, arg)));
            }
            out
        }
        Rhs::Tuple(items) => format!("tuple({})", join(items, operand)),
        Rhs::Array(items) => format!("array({})", join(items, operand)),
    }
}

fn arg(a: &Arg) -> String {
    match &a.name {
        Some(n) => format!("{}: {}", name(n), operand(&a.value)),
        None => operand(&a.value),
    }
}

fn operand(v: &Operand) -> String {
    match v {
        Operand::Copy(p) => format!("copy {}", place(p)),
        Operand::Move(p) => format!("move {}", place(p)),
        Operand::Borrow(p) => format!("borrow {}", place(p)),
        Operand::BorrowMut(p) => format!("borrow mut {}", place(p)),
        Operand::Const(c) => format!("const {}", constant(c)),
    }
}

/// Место в текстовой форме — для сообщений.
pub(crate) fn place(p: &Place) -> String {
    let mut out = name(&p.base);
    for s in &p.path {
        match s {
            Seg::Field(f) => {
                out.push('.');
                out.push_str(&name(f));
            }
            Seg::Pos(i) => out.push_str(&format!(".{}", i)),
            Seg::Elem(Index::Var(v)) => out.push_str(&format!("[{}]", name(v))),
            Seg::Elem(Index::Lit(n)) => out.push_str(&format!("[{}]", n)),
        }
    }
    out
}

fn constant(c: &Const) -> String {
    match c {
        Const::Int(n) => n.to_string(),
        Const::Float(bits) => format!("{:?}", f64::from_bits(*bits)),
        Const::Bool(b) => b.to_string(),
        Const::Char(ch) => format!("'{}'", escape(&ch.to_string(), '\'')),
        Const::Unit => "()".to_string(),
    }
}

fn string(s: &str) -> String {
    format!("\"{}\"", escape(s, '"'))
}

fn escape(s: &str, quote: char) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ───────── чтение ─────────

/// Ошибка чтения: строка и что не так.
#[derive(Debug, Clone, PartialEq)]
pub struct IrError {
    pub line: u32,
    pub message: String,
}

type Res<T> = Result<T, IrError>;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Слово без кавычек: имя или слово текстовой формы.
    Word(String),
    /// Имя в обратных кавычках.
    Quoted(String),
    Int(i128),
    Float(u64),
    Char(char),
    Str(String),
    Punct(&'static str),
    End,
}

struct Lexed {
    tok: Tok,
    line: u32,
}

fn err<T>(line: u32, message: String) -> Res<T> {
    Err(IrError { line, message })
}

fn lex(text: &str) -> Res<Vec<Lexed>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Lexed> = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == ';' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if ident_start(c) {
            let start = i;
            while i < chars.len() && ident_cont(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push(Lexed {
                tok: Tok::Word(word),
                line,
            });
        } else if c == '`' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != '`' && !chars[i].is_control() {
                i += 1;
            }
            if i >= chars.len() || chars[i] != '`' {
                return err(
                    line,
                    "имя в обратных кавычках не закрыто до конца строки или содержит управляющий символ"
                        .to_string(),
                );
            }
            if i == start {
                return err(line, "пустое имя в обратных кавычках".to_string());
            }
            let word: String = chars[start..i].iter().collect();
            i += 1;
            out.push(Lexed {
                tok: Tok::Quoted(word),
                line,
            });
        } else if c.is_ascii_digit()
            || (c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            // после точки — только целое: `t.0.1` — это два номера, а не дробь
            let after_dot = matches!(
                out.last(),
                Some(Lexed {
                    tok: Tok::Punct("."),
                    ..
                })
            );
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut float = false;
            if !after_dot {
                if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                    float = true;
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        float = true;
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
            }
            let digits: String = chars[start..i].iter().collect();
            if i < chars.len() && ident_cont(chars[i]) {
                return err(line, format!("число {} слито со словом", digits));
            }
            let tok = if float {
                match digits.parse::<f64>() {
                    Ok(v) if v.is_finite() => Tok::Float(v.to_bits()),
                    _ => return err(line, format!("число {} не помещается в f64", digits)),
                }
            } else {
                match digits.parse::<i128>() {
                    Ok(v) => Tok::Int(v),
                    Err(_) => return err(line, format!("целое {} слишком велико", digits)),
                }
            };
            out.push(Lexed { tok, line });
        } else if c == '\'' {
            let (text, next) = quoted(&chars, i, '\'', line)?;
            let mut it = text.chars();
            let ch = match (it.next(), it.next()) {
                (Some(ch), None) => ch,
                _ => {
                    return err(
                        line,
                        "в символьной константе должен быть ровно один символ".to_string(),
                    )
                }
            };
            out.push(Lexed {
                tok: Tok::Char(ch),
                line,
            });
            i = next;
        } else if c == '"' {
            let (text, next) = quoted(&chars, i, '"', line)?;
            out.push(Lexed {
                tok: Tok::Str(text),
                line,
            });
            i = next;
        } else {
            let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let p: &'static str = if two == "->" {
                "->"
            } else {
                match c {
                    '{' => "{",
                    '}' => "}",
                    '(' => "(",
                    ')' => ")",
                    '[' => "[",
                    ']' => "]",
                    ',' => ",",
                    ':' => ":",
                    '=' => "=",
                    '.' => ".",
                    _ => return err(line, format!("неожиданный знак «{}»", c)),
                }
            };
            i += p.chars().count();
            out.push(Lexed {
                tok: Tok::Punct(p),
                line,
            });
        }
    }
    out.push(Lexed {
        tok: Tok::End,
        line,
    });
    Ok(out)
}

/// Строка или символ в кавычках с экранированием; возвращает текст и позицию за кавычкой.
fn quoted(chars: &[char], start: usize, q: char, line: u32) -> Res<(String, usize)> {
    let mut out = String::new();
    let mut i = start + 1;
    loop {
        if i >= chars.len() || chars[i] == '\n' {
            return err(line, "кавычка не закрыта до конца строки".to_string());
        }
        let c = chars[i];
        if c == q {
            return Ok((out, i + 1));
        }
        if c != '\\' {
            out.push(c);
            i += 1;
            continue;
        }
        let e = match chars.get(i + 1) {
            Some(e) => *e,
            None => return err(line, "экранирование оборвано".to_string()),
        };
        i += 2;
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'u' => {
                if chars.get(i) != Some(&'{') {
                    return err(line, "после \\u нужна фигурная скобка".to_string());
                }
                let from = i + 1;
                let mut j = from;
                while j < chars.len() && chars[j].is_ascii_hexdigit() {
                    j += 1;
                }
                if j == from || chars.get(j) != Some(&'}') {
                    return err(line, "неверная запись \\u{…}".to_string());
                }
                let hex: String = chars[from..j].iter().collect();
                let code = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                match code {
                    Some(ch) => out.push(ch),
                    None => return err(line, format!("\\u{{{}}} — не символ Unicode", hex)),
                }
                i = j + 1;
            }
            other => return err(line, format!("неизвестное экранирование \\{}", other)),
        }
    }
}

/// Читает модуль из текстовой формы.
pub fn parse(text: &str) -> Res<Module> {
    let mut p = Parser {
        toks: lex(text)?,
        pos: 0,
    };
    p.module()
}

struct Parser {
    toks: Vec<Lexed>,
    pos: usize,
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Word(w) => format!("«{}»", w),
        Tok::Quoted(w) => format!("«`{}`»", w),
        Tok::Int(n) => format!("число {}", n),
        Tok::Float(b) => format!("число {:?}", f64::from_bits(*b)),
        Tok::Char(_) => "символьная константа".to_string(),
        Tok::Str(_) => "строка".to_string(),
        Tok::Punct(p) => format!("«{}»", p),
        Tok::End => "конец текста".to_string(),
    }
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, k: usize) -> &Tok {
        let i = (self.pos + k).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn line(&self) -> u32 {
        self.toks[self.pos].line
    }

    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn fail<T>(&self, what: &str) -> Res<T> {
        err(
            self.line(),
            format!("ожидалось: {}; стоит {}", what, describe(self.peek())),
        )
    }

    fn at_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Word(x) if x == w)
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.at_word(w) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_word(&mut self, w: &str) -> Res<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            self.fail(&format!("«{}»", w))
        }
    }

    fn at_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(x) if *x == p)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.at_punct(p) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> Res<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            self.fail(&format!("«{}»", p))
        }
    }

    fn at_name(&self) -> bool {
        match self.peek() {
            Tok::Word(w) => !reserved(w),
            Tok::Quoted(_) => true,
            _ => false,
        }
    }

    fn name(&mut self) -> Res<String> {
        match self.peek().clone() {
            Tok::Word(w) if !reserved(&w) => {
                self.advance();
                Ok(w)
            }
            Tok::Word(w) => err(
                self.line(),
                format!(
                    "«{}» — слово текстовой формы; такое имя пишут в обратных кавычках",
                    w
                ),
            ),
            Tok::Quoted(w) => {
                self.advance();
                Ok(w)
            }
            _ => self.fail("имя"),
        }
    }

    /// Составное имя функции: части через точку, сами части точек не содержат.
    fn func_name(&mut self) -> Res<String> {
        let mut out = self.segment()?;
        while self.eat_punct(".") {
            out.push('.');
            out.push_str(&self.segment()?);
        }
        Ok(out)
    }

    fn segment(&mut self) -> Res<String> {
        let line = self.line();
        let part = self.name()?;
        if part.contains('.') {
            return err(
                line,
                format!("«{}»: часть имени функции не может содержать точку", part),
            );
        }
        Ok(part)
    }

    fn int(&mut self) -> Res<i128> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                Ok(n)
            }
            _ => self.fail("целое число"),
        }
    }

    fn unsigned(&mut self, max: u64) -> Res<u64> {
        let line = self.line();
        let n = self.int()?;
        if n < 0 || n > max as i128 {
            return err(
                line,
                format!("число {} вне допустимых пределов 0…{}", n, max),
            );
        }
        Ok(n as u64)
    }

    fn string(&mut self) -> Res<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.advance();
                Ok(s)
            }
            _ => self.fail("строка"),
        }
    }

    // ── модуль и объявления ──

    fn module(&mut self) -> Res<Module> {
        self.expect_word("ir")?;
        let line = self.line();
        let version = self.int()?;
        if version != VERSION as i128 {
            return err(
                line,
                format!(
                    "версия IR {} не поддерживается; поддерживается {}",
                    version, VERSION
                ),
            );
        }
        let version = VERSION;
        let mut decls = Vec::new();
        while self.peek() != &Tok::End {
            decls.push(self.decl()?);
        }
        Ok(Module { version, decls })
    }

    fn decl(&mut self) -> Res<Decl> {
        let copy = self.eat_word("copy");
        if self.eat_word("struct") {
            let name = self.name()?;
            self.expect_punct("{")?;
            let fields = self.fields("}")?;
            self.expect_punct("}")?;
            return Ok(Decl::Struct(StructDecl { name, copy, fields }));
        }
        if self.eat_word("enum") {
            let name = self.name()?;
            self.expect_punct("{")?;
            let mut variants = Vec::new();
            if !self.at_punct("}") {
                variants.push(self.variant()?);
                while self.eat_punct(",") {
                    variants.push(self.variant()?);
                }
            }
            self.expect_punct("}")?;
            return Ok(Decl::Enum(EnumDecl {
                name,
                copy,
                variants,
            }));
        }
        if copy {
            return self.fail("«struct» или «enum» после «copy»");
        }
        if self.eat_word("func") {
            let name = self.func_name()?;
            self.expect_punct("(")?;
            let mut params = Vec::new();
            if !self.at_punct(")") {
                params.push(self.param()?);
                while self.eat_punct(",") {
                    params.push(self.param()?);
                }
            }
            self.expect_punct(")")?;
            let result = if self.eat_punct("->") {
                self.ty()?
            } else {
                Type::Unit
            };
            let mut uses = Vec::new();
            if self.eat_word("uses") {
                self.expect_punct("(")?;
                if !self.at_punct(")") {
                    uses.push(self.name()?);
                    while self.eat_punct(",") {
                        uses.push(self.name()?);
                    }
                }
                self.expect_punct(")")?;
            }
            let body = self.block()?;
            return Ok(Decl::Func(Func {
                name,
                params,
                result,
                uses,
                body,
            }));
        }
        if self.eat_word("test") {
            let name = self.string()?;
            let body = self.block()?;
            return Ok(Decl::Test(Test { name, body }));
        }
        self.fail("объявление: struct, enum, func или test")
    }

    /// Поля `имя: тип` через запятую до закрывающего знака (сам знак остаётся).
    fn fields(&mut self, close: &str) -> Res<Vec<Field>> {
        let mut fields = Vec::new();
        if self.at_punct(close) {
            return Ok(fields);
        }
        loop {
            let name = self.name()?;
            self.expect_punct(":")?;
            let ty = self.ty()?;
            fields.push(Field { name, ty });
            if !self.eat_punct(",") {
                return Ok(fields);
            }
        }
    }

    fn variant(&mut self) -> Res<Variant> {
        let name = self.name()?;
        if !self.eat_punct("(") {
            return Ok(Variant {
                name,
                payload: Payload::None,
            });
        }
        let named = self.at_name() && matches!(self.peek_at(1), Tok::Punct(":"));
        let payload = if named {
            Payload::Fields(self.fields(")")?)
        } else {
            Payload::One(self.ty()?)
        };
        self.expect_punct(")")?;
        Ok(Variant { name, payload })
    }

    fn param(&mut self) -> Res<Param> {
        let name = self.name()?;
        self.expect_punct(":")?;
        let ty = self.ty()?;
        let mode = if self.eat_word("mut") {
            Mode::Mut
        } else if self.eat_word("take") {
            Mode::Take
        } else {
            Mode::Read
        };
        Ok(Param { name, ty, mode })
    }

    fn ty(&mut self) -> Res<Type> {
        if let Tok::Word(w) = self.peek().clone() {
            if let Some(s) = Scalar::from_name(&w) {
                self.advance();
                return Ok(Type::Scalar(s));
            }
        }
        let line = self.line();
        if self.eat_punct("(") {
            if self.eat_punct(")") {
                return Ok(Type::Unit);
            }
            let mut items = vec![self.ty()?];
            while self.eat_punct(",") {
                items.push(self.ty()?);
            }
            self.expect_punct(")")?;
            if items.len() < 2 {
                return err(line, "у кортежа не меньше двух элементов".to_string());
            }
            return Ok(Type::Tuple(items));
        }
        if self.eat_word("Array") {
            self.expect_punct("[")?;
            let elem = self.ty()?;
            self.expect_punct(",")?;
            let n = self.unsigned(u64::MAX)?;
            self.expect_punct("]")?;
            return Ok(Type::Array(Box::new(elem), n));
        }
        if self.at_name() {
            return Ok(Type::Named(self.name()?));
        }
        self.fail("тип")
    }

    // ── инструкции ──

    fn block(&mut self) -> Res<Block> {
        self.expect_punct("{")?;
        let mut stmts = Vec::new();
        while !self.at_punct("}") {
            if self.peek() == &Tok::End {
                return self.fail("«}»");
            }
            stmts.push(self.stmt()?);
        }
        self.advance();
        Ok(Block { stmts })
    }

    fn state(&mut self) -> Res<State> {
        self.expect_punct("[")?;
        self.expect_word("held")?;
        self.expect_punct(":")?;
        let mut held = Vec::new();
        if !self.at_punct("]") {
            held.push(self.place()?);
            while self.eat_punct(",") {
                held.push(self.place()?);
            }
        }
        self.expect_punct("]")?;
        Ok(State { held })
    }

    fn after(&mut self) -> Res<Option<State>> {
        if self.at_punct("[") {
            Ok(Some(self.state()?))
        } else {
            Ok(None)
        }
    }

    fn stmt(&mut self) -> Res<Stmt> {
        if self.eat_word("local") {
            let name = self.name()?;
            self.expect_punct(":")?;
            let ty = self.ty()?;
            return Ok(Stmt::Local { name, ty });
        }
        if self.eat_word("drop") {
            return Ok(Stmt::Drop(self.place()?));
        }
        if self.eat_word("call") {
            let (func, args) = self.call_tail()?;
            return Ok(Stmt::Call { func, args });
        }
        if self.eat_word("if") {
            let cond = self.place()?;
            let then = self.block()?;
            self.expect_word("else")?;
            let otherwise = self.block()?;
            let after = self.after()?;
            return Ok(Stmt::If {
                cond,
                then,
                otherwise,
                after,
            });
        }
        if self.eat_word("switch") {
            let on = self.place()?;
            self.expect_punct("{")?;
            let mut cases = Vec::new();
            while self.eat_word("case") {
                let label = self.case_label()?;
                let body = self.block()?;
                cases.push(Case { label, body });
            }
            let otherwise = if self.eat_word("else") {
                Some(self.block()?)
            } else {
                None
            };
            self.expect_punct("}")?;
            let after = self.after()?;
            return Ok(Stmt::Switch {
                on,
                cases,
                otherwise,
                after,
            });
        }
        if self.eat_word("loop") {
            let label = self.name()?;
            let head = self.state()?;
            let body = self.block()?;
            let after = self.after()?;
            return Ok(Stmt::Loop {
                label,
                head,
                body,
                after,
            });
        }
        if self.eat_word("break") {
            return Ok(Stmt::Break(self.name()?));
        }
        if self.eat_word("continue") {
            return Ok(Stmt::Continue(self.name()?));
        }
        if self.eat_word("return") {
            if self.at_operand() {
                return Ok(Stmt::Return(Some(self.operand()?)));
            }
            return Ok(Stmt::Return(None));
        }
        if self.eat_word("crash") {
            return Ok(Stmt::Crash(self.string()?));
        }
        if !self.at_name() {
            return self.fail("инструкция");
        }
        if matches!(self.peek_at(1), Tok::Punct(":")) {
            let name = self.name()?;
            self.advance();
            let ty = self.ty()?;
            self.expect_punct("=")?;
            let value = self.rhs()?;
            return Ok(Stmt::Define { name, ty, value });
        }
        let place = self.place()?;
        self.expect_punct("=")?;
        let value = self.rhs()?;
        Ok(Stmt::Assign { place, value })
    }

    fn case_label(&mut self) -> Res<CaseLabel> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                Ok(CaseLabel::Value(Const::Int(n)))
            }
            Tok::Char(c) => {
                self.advance();
                Ok(CaseLabel::Value(Const::Char(c)))
            }
            Tok::Word(w) if w == "true" || w == "false" => {
                self.advance();
                Ok(CaseLabel::Value(Const::Bool(w == "true")))
            }
            _ if self.at_name() => Ok(CaseLabel::Variant(self.name()?)),
            _ => self.fail("вариант, целое, символ или логическое значение"),
        }
    }

    fn call_tail(&mut self) -> Res<(String, Vec<Operand>)> {
        let func = self.func_name()?;
        self.expect_punct("(")?;
        let args = self.operands()?;
        self.expect_punct(")")?;
        Ok((func, args))
    }

    /// Операнды через запятую до `)` (сама скобка остаётся).
    fn operands(&mut self) -> Res<Vec<Operand>> {
        let mut items = Vec::new();
        if self.at_punct(")") {
            return Ok(items);
        }
        items.push(self.operand()?);
        while self.eat_punct(",") {
            items.push(self.operand()?);
        }
        Ok(items)
    }

    fn rhs(&mut self) -> Res<Rhs> {
        if self.at_operand() {
            return Ok(Rhs::Use(self.operand()?));
        }
        if self.eat_word("call") {
            let (func, args) = self.call_tail()?;
            return Ok(Rhs::Call { func, args });
        }
        if self.eat_word("make") {
            let line = self.line();
            let ty = self.name()?;
            let variant = if self.eat_punct(".") {
                Some(self.name()?)
            } else {
                None
            };
            let args = if self.eat_punct("(") {
                let mut items = Vec::new();
                if !self.at_punct(")") {
                    items.push(self.arg()?);
                    while self.eat_punct(",") {
                        items.push(self.arg()?);
                    }
                }
                self.expect_punct(")")?;
                Some(items)
            } else {
                None
            };
            if variant.is_none() && args.is_none() {
                return err(
                    line,
                    "запись создаётся с полями в скобках: make T(…)".to_string(),
                );
            }
            return Ok(Rhs::Make { ty, variant, args });
        }
        if self.eat_word("tuple") {
            let line = self.line();
            self.expect_punct("(")?;
            let items = self.operands()?;
            self.expect_punct(")")?;
            if items.len() < 2 {
                return err(line, "у кортежа не меньше двух элементов".to_string());
            }
            return Ok(Rhs::Tuple(items));
        }
        if self.eat_word("array") {
            self.expect_punct("(")?;
            let items = self.operands()?;
            self.expect_punct(")")?;
            return Ok(Rhs::Array(items));
        }
        if self.eat_word("conv") {
            self.expect_punct(".")?;
            let from = self.scalar()?;
            self.expect_punct(".")?;
            let to = self.scalar()?;
            self.expect_punct("(")?;
            let arg = self.operand()?;
            self.expect_punct(")")?;
            return Ok(Rhs::Conv { from, to, arg });
        }
        if let Tok::Word(w) = self.peek().clone() {
            if let Some(op) = Op::from_name(&w) {
                let line = self.line();
                self.advance();
                self.expect_punct(".")?;
                let ty = self.scalar()?;
                self.expect_punct("(")?;
                let args = self.operands()?;
                self.expect_punct(")")?;
                if args.len() != op.arity() {
                    return err(
                        line,
                        format!(
                            "операция {} берёт операндов: {}; дано: {}",
                            op.name(),
                            op.arity(),
                            args.len()
                        ),
                    );
                }
                return Ok(Rhs::Op { op, ty, args });
            }
        }
        self.fail(
            "значение: copy, move, borrow, const, call, make, tuple, array, conv или операция",
        )
    }

    fn scalar(&mut self) -> Res<Scalar> {
        if let Tok::Word(w) = self.peek().clone() {
            if let Some(s) = Scalar::from_name(&w) {
                self.advance();
                return Ok(s);
            }
        }
        self.fail("скаляр")
    }

    fn arg(&mut self) -> Res<Arg> {
        if self.at_name() && matches!(self.peek_at(1), Tok::Punct(":")) {
            let name = self.name()?;
            self.advance();
            let value = self.operand()?;
            return Ok(Arg {
                name: Some(name),
                value,
            });
        }
        Ok(Arg {
            name: None,
            value: self.operand()?,
        })
    }

    fn at_operand(&self) -> bool {
        ["copy", "move", "borrow", "const"]
            .iter()
            .any(|w| self.at_word(w))
    }

    fn operand(&mut self) -> Res<Operand> {
        if self.eat_word("copy") {
            return Ok(Operand::Copy(self.place()?));
        }
        if self.eat_word("move") {
            return Ok(Operand::Move(self.place()?));
        }
        if self.eat_word("borrow") {
            if self.eat_word("mut") {
                return Ok(Operand::BorrowMut(self.place()?));
            }
            return Ok(Operand::Borrow(self.place()?));
        }
        if self.eat_word("const") {
            return Ok(Operand::Const(self.constant()?));
        }
        self.fail("операнд: copy, move, borrow или const")
    }

    fn constant(&mut self) -> Res<Const> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                Ok(Const::Int(n))
            }
            Tok::Float(b) => {
                self.advance();
                Ok(Const::Float(b))
            }
            Tok::Char(c) => {
                self.advance();
                Ok(Const::Char(c))
            }
            Tok::Word(w) if w == "true" || w == "false" => {
                self.advance();
                Ok(Const::Bool(w == "true"))
            }
            Tok::Punct("(") => {
                self.advance();
                self.expect_punct(")")?;
                Ok(Const::Unit)
            }
            _ => self.fail("константа"),
        }
    }

    fn place(&mut self) -> Res<Place> {
        let base = self.name()?;
        let mut path = Vec::new();
        loop {
            if self.eat_punct(".") {
                if let Tok::Int(_) = self.peek() {
                    path.push(Seg::Pos(self.unsigned(u32::MAX as u64)? as u32));
                } else {
                    path.push(Seg::Field(self.name()?));
                }
            } else if self.eat_punct("[") {
                if let Tok::Int(_) = self.peek() {
                    path.push(Seg::Elem(Index::Lit(self.unsigned(u64::MAX)?)));
                } else {
                    path.push(Seg::Elem(Index::Var(self.name()?)));
                }
                self.expect_punct("]")?;
            } else {
                return Ok(Place { base, path });
            }
        }
    }
}
