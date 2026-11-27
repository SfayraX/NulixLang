//! Каноническая запись синтаксического дерева: `(вид часть часть …)`.
//!
//! Ту же запись выдаёт проверочный разборщик по дереву Lark (`tools/grammar/canon.py`).
//! Сверка двух записей на общих примерах — в `tests/parser_golden.rs`.
//! В запись не входят скобки вокруг типа, завершающие запятые и концы инструкций.

use crate::ast::*;

struct Node {
    text: String,
}

impl Node {
    fn new(head: &str) -> Node {
        let mut text = String::from("(");
        text.push_str(head);
        Node { text }
    }

    fn add(&mut self, part: &str) {
        self.text.push(' ');
        self.text.push_str(part);
    }

    fn done(mut self) -> String {
        self.text.push(')');
        self.text
    }
}

/// `(вид часть)`.
fn pair(head: &str, part: &str) -> String {
    let mut n = Node::new(head);
    n.add(part);
    n.done()
}

/// `(вид имя имя …)`.
fn names(head: &str, list: &[String]) -> String {
    let mut n = Node::new(head);
    for name in list {
        n.add(name);
    }
    n.done()
}

fn mode(m: Mode) -> String {
    match m {
        Mode::Mut => String::from("(mode mut)"),
        Mode::Take => String::from("(mode take)"),
    }
}

// ───────── файл ─────────

pub fn file(f: &File) -> String {
    let mut n = Node::new("file");
    if let Some(args) = &f.header {
        n.add(&header(args));
    }
    for item in &f.imports {
        n.add(&import(item));
    }
    for item in &f.items {
        match item {
            TopItem::Decl(d) => n.add(&decl(d)),
            TopItem::Stmt(s) => n.add(&stmt(s)),
        }
    }
    n.done()
}

pub fn header(args: &[HeaderArg]) -> String {
    let mut n = Node::new("header");
    for arg in args {
        match arg {
            HeaderArg::Name(name) => n.add(&pair("harg", name)),
            HeaderArg::Target(name) => n.add(&pair("harg-target", name)),
        }
    }
    n.done()
}

pub fn import(i: &Import) -> String {
    let mut n = Node::new("import");
    n.add(&names("path", &i.path));
    match &i.tail {
        ImportTail::Module(None) => {}
        ImportTail::Module(Some(alias)) => n.add(&pair("as", alias)),
        ImportTail::Names(list) => {
            let mut group = Node::new("group");
            for item in list {
                let mut x = Node::new("item");
                x.add(&item.name);
                if let Some(alias) = &item.alias {
                    x.add(&pair("as", alias));
                }
                group.add(&x.done());
            }
            n.add(&group.done());
        }
    }
    n.done()
}

// ───────── объявления ─────────

pub fn decl(d: &Decl) -> String {
    let mut n = Node::new("decl");
    for a in &d.attrs {
        n.add(&attr(a));
    }
    if d.is_pub {
        n.add("pub");
    }
    let core = match &d.kind {
        DeclKind::Binding(b) => binding(b),
        DeclKind::Func(f) => func_decl(f),
        DeclKind::ExternBlock(b) => extern_block(b),
        DeclKind::Struct(s) => struct_decl(s),
        DeclKind::Enum(e) => enum_decl(e),
        DeclKind::Interface(i) => interface_decl(i),
        DeclKind::Impl(i) => impl_decl(i),
        DeclKind::TypeAlias(t) => type_alias(t),
        DeclKind::Test(t) => test_decl(t),
    };
    n.add(&core);
    n.done()
}

pub fn attr(a: &Attr) -> String {
    match a {
        Attr::Level(marker) => pair("level", marker),
        Attr::Strictness(marker) => pair("strictness", marker),
        Attr::Custom(name, None) => pair("attr", name),
        Attr::Custom(name, Some(list)) => {
            let mut args = Node::new("args");
            for x in list {
                args.add(&arg(x));
            }
            let mut n = Node::new("attr");
            n.add(name);
            n.add(&args.done());
            n.done()
        }
    }
}

fn binding(b: &Binding) -> String {
    let mut n = Node::new(match b.kind {
        BindKind::Let => "let",
        BindKind::Var => "var",
        BindKind::Const => "const",
    });
    match &b.target {
        BindTarget::Name(name) => n.add(&pair("bind", name)),
        BindTarget::Tuple(p) => n.add(&pattern(p)),
    }
    if let Some(t) = &b.ty {
        n.add(&pair(":", &ty(t)));
    }
    n.add(&expr(&b.value));
    n.done()
}

fn func_decl(f: &FuncDecl) -> String {
    let mut n = match &f.abi {
        None => Node::new("func"),
        Some(abi) => {
            let mut n = Node::new("extern-func");
            n.add(&string(abi));
            n
        }
    };
    n.add(&func_sig(&f.sig));
    n.add(&block(&f.body));
    if let Some(name) = &f.end {
        n.add(&pair("end", name));
    }
    n.done()
}

pub fn func_sig(s: &FuncSig) -> String {
    let mut n = Node::new("sig");
    n.add(&s.name);
    if !s.generics.is_empty() {
        n.add(&generics(&s.generics));
    }
    n.add(&params(&s.params));
    add_tail(&mut n, &s.ret, &s.reqs);
    n.done()
}

/// Результат и требования после параметров: `[(-> тип)] [(reqs …)]`.
fn add_tail(n: &mut Node, ret: &Option<Type>, rq: &Option<Reqs>) {
    if let Some(t) = ret {
        n.add(&pair("->", &ty(t)));
    }
    if let Some(r) = rq {
        n.add(&reqs(r));
    }
}

pub fn sig_tail(ret: &Option<Type>, rq: &Option<Reqs>) -> String {
    let mut n = Node::new("sig-tail");
    add_tail(&mut n, ret, rq);
    n.done()
}

fn params(list: &[Param]) -> String {
    let mut n = Node::new("params");
    for p in list {
        n.add(&param(p));
    }
    n.done()
}

pub fn param(p: &Param) -> String {
    let mut n = match &p.kind {
        ParamKind::SelfValue => Node::new("self-param"),
        ParamKind::Named(_, _) => Node::new("param"),
    };
    if let Some(m) = p.mode {
        n.add(&mode(m));
    }
    if let ParamKind::Named(name, t) = &p.kind {
        n.add(name);
        if let Some(t) = t {
            n.add(&pair(":", &ty(t)));
        }
    }
    n.done()
}

pub fn generics(list: &[GenericParam]) -> String {
    let mut n = Node::new("generics");
    for g in list {
        match g {
            GenericParam::Type(name, bounds) => {
                let mut x = Node::new("tparam");
                x.add(name);
                if !bounds.is_empty() {
                    x.add(&bound(bounds));
                }
                n.add(&x.done());
            }
            GenericParam::Reqs(name, without) => {
                let mut x = Node::new("rparam");
                x.add(name);
                if let Some(items) = without {
                    x.add(&req_items("without", items));
                }
                n.add(&x.done());
            }
            GenericParam::Life(name) => n.add(&pair("lparam", name)),
        }
    }
    n.done()
}

pub fn bound(list: &[PathType]) -> String {
    let mut n = Node::new("bound");
    for p in list {
        n.add(&path_type(p));
    }
    n.done()
}

pub fn reqs(r: &Reqs) -> String {
    let mut n = Node::new("reqs");
    if let Some(items) = &r.uses {
        n.add(&req_items("uses", items));
    }
    if let Some(items) = &r.without {
        n.add(&req_items("without", items));
    }
    if !r.limit.is_empty() {
        let mut limit = Node::new("limit");
        for item in &r.limit {
            let mut x = Node::new(if item.at_most { "limit-le" } else { "limit-ge" });
            x.add(&item.name);
            x.add(&expr(&item.value));
            limit.add(&x.done());
        }
        n.add(&limit.done());
    }
    n.done()
}

fn req_items(head: &str, items: &[ReqItem]) -> String {
    let mut n = Node::new(head);
    for item in items {
        match item {
            ReqItem::Name(list) => n.add(&names("req", list)),
            ReqItem::Trust => n.add("(req-trust)"),
        }
    }
    n.done()
}

fn struct_decl(s: &StructDecl) -> String {
    let mut n = Node::new("struct");
    n.add(&s.name);
    if !s.generics.is_empty() {
        n.add(&generics(&s.generics));
    }
    for f in &s.fields {
        n.add(&field(f));
    }
    if let Some(name) = &s.end {
        n.add(&pair("end", name));
    }
    n.done()
}

pub fn field(f: &Field) -> String {
    let mut n = Node::new("field");
    n.add(&f.name);
    for a in &f.attrs {
        n.add(&attr(a));
    }
    if f.is_pub {
        n.add("pub");
    }
    n.add(&ty(&f.ty));
    if let Some(e) = &f.default {
        n.add(&pair("default", &expr(e)));
    }
    n.done()
}

fn enum_decl(e: &EnumDecl) -> String {
    let mut n = Node::new("enum");
    n.add(&e.name);
    if !e.generics.is_empty() {
        n.add(&generics(&e.generics));
    }
    for v in &e.variants {
        n.add(&variant(v));
    }
    if let Some(name) = &e.end {
        n.add(&pair("end", name));
    }
    n.done()
}

pub fn variant(v: &Variant) -> String {
    let mut n = Node::new("variant");
    n.add(&v.name);
    if v.indirect {
        n.add("indirect");
    }
    match &v.fields {
        VariantFields::None => {}
        VariantFields::Positional(t) => n.add(&pair("positional", &ty(t))),
        VariantFields::Named(list) => {
            let mut named = Node::new("named");
            for f in list {
                let mut x = Node::new("nf");
                x.add(&f.name);
                x.add(&ty(&f.ty));
                named.add(&x.done());
            }
            n.add(&named.done());
        }
    }
    n.done()
}

fn interface_decl(i: &InterfaceDecl) -> String {
    let mut n = Node::new("interface");
    n.add(&i.name);
    if !i.generics.is_empty() {
        n.add(&generics(&i.generics));
    }
    if !i.bound.is_empty() {
        n.add(&bound(&i.bound));
    }
    for m in &i.members {
        n.add(&member(m));
    }
    if let Some(name) = &i.end {
        n.add(&pair("end", name));
    }
    n.done()
}

fn impl_decl(i: &ImplDecl) -> String {
    let mut n = Node::new("impl");
    if !i.generics.is_empty() {
        n.add(&generics(&i.generics));
    }
    n.add(&path_type(&i.first));
    if let Some(p) = &i.for_type {
        n.add(&pair("for", &path_type(p)));
    }
    for m in &i.members {
        n.add(&member(m));
    }
    if let Some(end) = &i.end {
        let mut x = Node::new("end-impl");
        x.add(&names("tname", &end.first));
        if let Some(list) = &end.for_type {
            x.add(&pair("for", &names("tname", list)));
        }
        n.add(&x.done());
    }
    n.done()
}

pub fn member(m: &Member) -> String {
    let mut n = Node::new("member");
    for a in &m.attrs {
        n.add(&attr(a));
    }
    if m.is_pub {
        n.add("pub");
    }
    match &m.kind {
        MemberKind::Sig(s) => n.add(&func_sig(s)),
        MemberKind::Func(f) => n.add(&func_decl(f)),
        MemberKind::Binding(b) => n.add(&binding(b)),
    }
    n.done()
}

fn type_alias(t: &TypeAlias) -> String {
    let mut n = Node::new("type-alias");
    n.add(&t.name);
    if !t.generics.is_empty() {
        n.add(&generics(&t.generics));
    }
    n.add(&ty(&t.ty));
    n.done()
}

fn test_decl(t: &TestDecl) -> String {
    let mut n = Node::new("test");
    n.add(&string(&t.name));
    n.add(&block(&t.body));
    if t.has_end {
        n.add("end");
    }
    n.done()
}

fn extern_block(b: &ExternBlock) -> String {
    let mut n = Node::new("extern-block");
    n.add(&string(&b.abi));
    n.add(&string(&b.reason));
    for m in &b.members {
        n.add(&member(m));
    }
    if b.has_end {
        n.add("end");
    }
    n.done()
}

/// Строка: написание целиком, затем подстановки по порядку.
fn string(s: &StrLit) -> String {
    let mut n = Node::new("str");
    n.add(&s.raw);
    for sub in &s.subs {
        match sub {
            StrSub::Name(name) => n.add(&pair("sub-name", name)),
            StrSub::Expr(e) => n.add(&pair("sub", &expr(e))),
        }
    }
    n.done()
}

// ───────── типы ─────────

pub fn ty(t: &Type) -> String {
    match &t.kind {
        TypeKind::Path(p) => path_type(p),
        TypeKind::Any(None) => String::from("(t-any)"),
        TypeKind::Any(Some(p)) => pair("t-any", &path_type(p)),
        TypeKind::Ref(mutable, origin, inner) => {
            let mut n = Node::new(if *mutable { "t-mutref" } else { "t-ref" });
            match origin {
                None => {}
                Some(Origin::Name(name)) => n.add(&pair("origin", name)),
                Some(Origin::SelfValue) => n.add("(origin-self)"),
            }
            n.add(&ty(inner));
            n.done()
        }
        TypeKind::Ptr(mutable, inner) => {
            pair(if *mutable { "t-mutptr" } else { "t-ptr" }, &ty(inner))
        }
        TypeKind::Unit => String::from("(t-unit)"),
        TypeKind::Tuple(list) => {
            let mut n = Node::new("t-tuple");
            for x in list {
                n.add(&ty(x));
            }
            n.done()
        }
        TypeKind::Func(f) => func_type(f),
    }
}

fn path_type(p: &PathType) -> String {
    let mut n = Node::new("t-path");
    for name in &p.names {
        n.add(name);
    }
    if !p.args.is_empty() {
        n.add(&generic_args(&p.args));
    }
    n.done()
}

pub fn generic_args(list: &[GenericArg]) -> String {
    let mut n = Node::new("targs");
    for a in list {
        match a {
            GenericArg::Type(t) => n.add(&ty(t)),
            GenericArg::Int(value) => n.add(&pair("int", value)),
            GenericArg::Uses(items) => n.add(&req_items("uses", items)),
            GenericArg::Life(LifeSource::SelfValue) => n.add("(life-self)"),
            GenericArg::Life(LifeSource::Names(list)) => n.add(&names("life", list)),
        }
    }
    n.done()
}

fn func_type(f: &FuncType) -> String {
    let mut n = Node::new("t-func");
    match &f.kind {
        None => {}
        Some(FuncKind::Mut) => n.add("(kind mut)"),
        Some(FuncKind::Take) => n.add("(kind take)"),
        Some(FuncKind::Any) => n.add("(kind any)"),
        Some(FuncKind::Extern(abi)) => {
            let mut kind = Node::new("kind");
            kind.add("extern");
            kind.add(&string(abi));
            n.add(&kind.done());
        }
    }
    let mut list = Node::new("params");
    for p in &f.params {
        let mut x = Node::new("fparam");
        if let Some(m) = p.mode {
            x.add(&mode(m));
        }
        if let Some(name) = &p.name {
            x.add(name);
        }
        x.add(&ty(&p.ty));
        list.add(&x.done());
    }
    n.add(&list.done());
    add_tail(&mut n, &f.ret, &f.reqs);
    n.done()
}

// ───────── инструкции ─────────

pub fn block(b: &Block) -> String {
    let mut n = Node::new("block");
    for s in &b.stmts {
        n.add(&stmt(s));
    }
    n.done()
}

pub fn stmts(list: &[Stmt]) -> String {
    let mut n = Node::new("stmts");
    for s in list {
        n.add(&stmt(s));
    }
    n.done()
}

pub fn stmt(s: &Stmt) -> String {
    match &s.kind {
        StmtKind::Binding(b) => binding(b),
        StmtKind::Expr(e) => expr(e),
        StmtKind::Assign(op, target, value) => {
            let mut n = Node::new("assign");
            match op {
                None => n.add("="),
                Some(op) => n.add(&format!("{}=", op.text())),
            }
            n.add(&expr(target));
            n.add(&expr(value));
            n.done()
        }
        StmtKind::For(f) => {
            let mut n = Node::new("for");
            add_label(&mut n, &f.label);
            match &f.target {
                ForTarget::Name(name) => n.add(&pair("bind", name)),
                ForTarget::Wildcard => n.add("(p-wild)"),
                ForTarget::Tuple(p) => n.add(&pattern(p)),
            }
            if let Some(m) = f.mode {
                n.add(&mode(m));
            }
            n.add(&expr(&f.iter));
            n.add(&block(&f.body));
            add_end_loop(&mut n, &f.end);
            n.done()
        }
        StmtKind::While(w) => {
            let mut n = Node::new("while");
            add_label(&mut n, &w.label);
            n.add(&expr(&w.cond));
            n.add(&block(&w.body));
            add_end_loop(&mut n, &w.end);
            n.done()
        }
        StmtKind::Loop(l) => {
            let mut n = Node::new("loop");
            add_label(&mut n, &l.label);
            n.add(&block(&l.body));
            add_end_loop(&mut n, &l.end);
            n.done()
        }
        StmtKind::Return(None) => String::from("(return)"),
        StmtKind::Return(Some(e)) => pair("return", &expr(e)),
        StmtKind::Break(None) => String::from("(break)"),
        StmtKind::Break(Some(label)) => pair("break", label),
        StmtKind::Continue(None) => String::from("(continue)"),
        StmtKind::Continue(Some(label)) => pair("continue", label),
        StmtKind::Assert(cond, message) => {
            let mut n = Node::new("assert");
            n.add(&expr(cond));
            if let Some(e) = message {
                n.add(&expr(e));
            }
            n.done()
        }
        StmtKind::DiscardSelf => String::from("(discard-self)"),
    }
}

fn add_label(n: &mut Node, label: &Option<String>) {
    if let Some(name) = label {
        n.add(&pair("label", name));
    }
}

fn add_end_loop(n: &mut Node, end: &Option<EndLoop>) {
    match end {
        None => {}
        Some(EndLoop::Keyword(word)) => n.add(&pair("end-kw", word)),
        Some(EndLoop::Label(name)) => n.add(&pair("end", name)),
    }
}

// ───────── выражения ─────────

pub fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Name(name) => pair("name", name),
        ExprKind::SelfValue => String::from("(self)"),
        ExprKind::Int(value) => pair("int", value),
        ExprKind::Float(value) => pair("float", value),
        ExprKind::Str(s) => string(s),
        ExprKind::Char(value) => pair("char", value),
        ExprKind::Bool(true) => String::from("(true)"),
        ExprKind::Bool(false) => String::from("(false)"),
        ExprKind::Unit => String::from("(unit)"),
        ExprKind::Paren(inner) => pair("paren", &expr(inner)),
        ExprKind::Tuple(list) => exprs("tuple", list),
        ExprKind::List(list) => exprs("list", list),
        ExprKind::Binary(first, rest) => {
            // запись вложенная, как у проверочного разборщика: (op2 (op1 A B) C)
            let mut text = String::new();
            for (op, _) in rest.iter().rev() {
                text.push('(');
                text.push_str(op.text());
                text.push(' ');
            }
            text.push_str(&expr(first));
            for (_, operand) in rest {
                text.push(' ');
                text.push_str(&expr(operand));
                text.push(')');
            }
            text
        }
        ExprKind::Unary(op, inner) => {
            let head = match op {
                UnOp::Not => "not",
                UnOp::Neg => "neg",
                UnOp::BitNot => "bitnot",
                UnOp::Try => "try",
                UnOp::Await => "await",
            };
            pair(head, &expr(inner))
        }
        ExprKind::Is(value, p) => {
            let mut n = Node::new("is");
            n.add(&expr(value));
            n.add(&pattern(p));
            n.done()
        }
        ExprKind::Catch(c) => {
            let mut n = Node::new("catch");
            n.add(&expr(&c.value));
            if let Some(name) = &c.name {
                n.add(&pair("err", name));
            }
            n.add(&block(&c.body));
            add_end(&mut n, c.has_end);
            n.done()
        }
        ExprKind::Postfix(base, ops) => {
            // запись вложенная: (index (call (. A имя) аргументы) индексы)
            let mut text = String::new();
            for op in ops.iter().rev() {
                text.push_str(match op {
                    PostfixOp::Field(_) => "(. ",
                    PostfixOp::Call(_) => "(call ",
                    PostfixOp::Index(_) => "(index ",
                });
            }
            text.push_str(&expr(base));
            for op in ops {
                match op {
                    PostfixOp::Field(name) => {
                        text.push(' ');
                        text.push_str(name);
                    }
                    PostfixOp::Call(args) => {
                        for a in args {
                            text.push(' ');
                            text.push_str(&arg(a));
                        }
                    }
                    PostfixOp::Index(args) => {
                        for a in args {
                            text.push(' ');
                            text.push_str(&index_arg(a));
                        }
                    }
                }
                text.push(')');
            }
            text
        }
        ExprKind::If(i) => if_expr(i),
        ExprKind::Match(m) => {
            let mut n = Node::new("match");
            if let Some(md) = m.mode {
                n.add(&mode(md));
            }
            n.add(&expr(&m.value));
            for item in &m.items {
                n.add(&match_item(item));
            }
            add_end(&mut n, m.has_end);
            n.done()
        }
        ExprKind::Closure(c) => {
            let mut n = Node::new("closure");
            n.add(&params(&c.params));
            add_tail(&mut n, &c.ret, &c.reqs);
            n.add(&block(&c.body));
            add_end(&mut n, c.has_end);
            n.done()
        }
        ExprKind::Trust(t) => {
            let mut n = Node::new("trust");
            n.add(&string(&t.reason));
            n.add(&block(&t.body));
            add_end(&mut n, t.has_end);
            n.done()
        }
        ExprKind::Provide(p) => {
            let mut n = Node::new("provide");
            for item in &p.items {
                let mut x = Node::new("pitem");
                x.add(&item.name);
                x.add(&expr(&item.value));
                n.add(&x.done());
            }
            n.add(&block(&p.body));
            add_end(&mut n, p.has_end);
            n.done()
        }
        ExprKind::Target(t) => {
            let mut n = Node::new("target");
            for item in &t.items {
                match item {
                    TargetItem::Case(name, body, has_end) => {
                        let mut x = Node::new("case");
                        x.add(&names("req", name));
                        x.add(&block(body));
                        add_end(&mut x, *has_end);
                        n.add(&x.done());
                    }
                    TargetItem::Else(body, has_end) => {
                        let mut x = Node::new("case-else");
                        x.add(&block(body));
                        add_end(&mut x, *has_end);
                        n.add(&x.done());
                    }
                }
            }
            add_end(&mut n, t.has_end);
            n.done()
        }
        ExprKind::LevelBlock(l) => {
            let mut n = Node::new("level-block");
            n.add(&l.marker);
            n.add(&block(&l.body));
            if let Some(marker) = &l.end {
                n.add(&pair("end", marker));
            }
            n.done()
        }
    }
}

fn exprs(head: &str, list: &[Expr]) -> String {
    let mut n = Node::new(head);
    for e in list {
        n.add(&expr(e));
    }
    n.done()
}

/// Слово `end` в конце записи: у блока была метка.
fn add_end(n: &mut Node, has_end: bool) {
    if has_end {
        n.add("end");
    }
}

fn arg(a: &Arg) -> String {
    match a {
        Arg::Plain(e) => expr(e),
        Arg::Mut(e) => pair("mut-arg", &expr(e)),
        Arg::Named(name, e) => {
            let mut n = Node::new("named-arg");
            n.add(name);
            n.add(&expr(e));
            n.done()
        }
    }
}

fn index_arg(a: &IndexArg) -> String {
    match a {
        IndexArg::Expr(e) => expr(e),
        IndexArg::Any(None) => String::from("(t-any)"),
        IndexArg::Any(Some(p)) => pair("t-any", &path_type(p)),
        IndexArg::Uses(items) => req_items("uses", items),
    }
}

/// Запись вложенная: `(if C1 B1 (else (if C2 B2 (else B3))) end)`.
fn if_expr(i: &IfExpr) -> String {
    let mut text = String::new();
    for (k, branch) in i.branches.iter().enumerate() {
        text.push_str(if k == 0 { "(if " } else { " (else (if " });
        text.push_str(&expr(&branch.cond));
        text.push(' ');
        text.push_str(&block(&branch.then));
    }
    if let Some(b) = &i.otherwise {
        text.push_str(" (else ");
        text.push_str(&block(b));
        text.push(')');
    }
    for _ in 1..i.branches.len() {
        text.push_str("))");
    }
    if i.has_end {
        text.push_str(" end");
    }
    text.push(')');
    text
}

pub fn match_item(item: &MatchItem) -> String {
    match item {
        MatchItem::Else(body, has_end, _) => {
            let mut n = Node::new("case-else");
            n.add(&block(body));
            add_end(&mut n, *has_end);
            n.done()
        }
        MatchItem::Case(c) => {
            let mut pats = Node::new("pats");
            for p in &c.patterns {
                pats.add(&pattern(p));
            }
            let mut n = Node::new("case");
            n.add(&pats.done());
            if let Some(g) = &c.guard {
                n.add(&pair("guard", &expr(g)));
            }
            n.add(&block(&c.body));
            add_end(&mut n, c.has_end);
            n.done()
        }
    }
}

// ───────── образцы ─────────

pub fn pattern(p: &Pattern) -> String {
    match &p.kind {
        PatternKind::Wildcard => String::from("(p-wild)"),
        PatternKind::Int(false, value) => pair("p-int", value),
        PatternKind::Int(true, value) => pair("p-neg-int", value),
        PatternKind::Float(false, value) => pair("p-float", value),
        PatternKind::Float(true, value) => pair("p-neg-float", value),
        PatternKind::Str(s) => pair("p-str", &string(s)),
        PatternKind::Char(value) => pair("p-char", value),
        PatternKind::Bool(true) => String::from("(p-true)"),
        PatternKind::Bool(false) => String::from("(p-false)"),
        PatternKind::Path(path) => names("p-path", path),
        PatternKind::Ctor(path, args) => {
            let mut n = Node::new("p-ctor");
            n.add(&names("p-path", path));
            for a in args {
                match a {
                    PatternArg::Pattern(inner) => n.add(&pattern(inner)),
                    PatternArg::Field(name, inner) => {
                        let mut x = Node::new("p-field");
                        x.add(name);
                        x.add(&pattern(inner));
                        n.add(&x.done());
                    }
                    PatternArg::Rest => n.add("(p-rest)"),
                }
            }
            n.done()
        }
        PatternKind::Tuple(list) => {
            let mut n = Node::new("p-tuple");
            for x in list {
                n.add(&pattern(x));
            }
            n.done()
        }
    }
}
