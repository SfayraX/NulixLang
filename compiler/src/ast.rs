//! Синтаксическое дерево Nulix: то, что разборщик строит по тексту программы.
//!
//! Дерево повторяет принятую грамматику (`grammar/nulix.lark`) и ничего не проверяет:
//! имена, типы и совпадение меток `end` сверяются на следующих шагах.
//!
//! Цепочки — `a + b + c`, `x.f().g()`, `if … else if …` — хранятся списками, а не вложением,
//! поэтому глубина дерева ограничена пределом вложенности, какой бы длины ни была цепочка.

/// Целый файл: заголовок, импорты, объявления и инструкции верхнего уровня.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub header: Option<Vec<HeaderArg>>,
    /// Строка заголовка `@file(…)`; без заголовка — 0.
    pub header_line: u32,
    pub imports: Vec<Import>,
    pub items: Vec<TopItem>,
}

/// Часть заголовка `@file(…)`: слово или `target(имя)`.
#[derive(Debug, Clone, PartialEq)]
pub enum HeaderArg {
    Name(String),
    Target(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: Vec<String>,
    pub tail: ImportTail,
    pub line: u32,
}

/// Чем кончается импорт: модуль целиком (с новым именем или без) или список имён в фигурных скобках.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportTail {
    Module(Option<String>),
    Names(Vec<ImportName>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportName {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopItem {
    Decl(Decl),
    Stmt(Stmt),
}

// ───────── объявления ─────────

#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub kind: DeclKind,
    pub line: u32,
}

/// Пометка перед объявлением: уровень, строгость или пометка с именем и аргументами.
#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    Level(String),
    Strictness(String),
    Custom(String, Option<Vec<Arg>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeclKind {
    Binding(Binding),
    Func(FuncDecl),
    ExternBlock(ExternBlock),
    Struct(StructDecl),
    Enum(EnumDecl),
    Interface(InterfaceDecl),
    Impl(ImplDecl),
    TypeAlias(TypeAlias),
    Test(TestDecl),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindKind {
    Let,
    Var,
    Const,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub kind: BindKind,
    pub target: BindTarget,
    pub ty: Option<Type>,
    pub value: Expr,
    pub line: u32,
}

/// Что связывается: одно имя или образец-кортеж.
#[derive(Debug, Clone, PartialEq)]
pub enum BindTarget {
    Name(String),
    Tuple(Pattern),
}

/// Функция с телом. `abi` задано у функций вида `extern "C" func …`.
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDecl {
    pub abi: Option<StrLit>,
    pub sig: FuncSig,
    pub body: Block,
    pub end: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncSig {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub reqs: Option<Reqs>,
    pub line: u32,
}

/// Параметр в квадратных скобках: тип с границей, переменная требований или время жизни.
#[derive(Debug, Clone, PartialEq)]
pub enum GenericParam {
    Type(String, Vec<PathType>),
    Reqs(String, Option<Vec<ReqItem>>),
    Life(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Mut,
    Take,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub mode: Option<Mode>,
    pub kind: ParamKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Named(String, Option<Type>),
    SelfValue,
}

/// Требования функции: `uses`, `without` и `limit`. Пустой `limit` значит, что его нет.
#[derive(Debug, Clone, PartialEq)]
pub struct Reqs {
    pub uses: Option<Vec<ReqItem>>,
    pub without: Option<Vec<ReqItem>>,
    pub limit: Vec<LimitItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReqItem {
    Name(Vec<String>),
    Trust,
}

/// Один предел: `имя <= значение` (`at_most`) или `имя >= значение`.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitItem {
    pub name: String,
    pub at_most: bool,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<Field>,
    pub end: Option<String>,
    /// Строка слова `struct`.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub variants: Vec<Variant>,
    pub end: Option<String>,
    /// Строка слова `enum`.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub indirect: bool,
    pub name: String,
    pub fields: VariantFields,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VariantFields {
    None,
    Positional(Type),
    Named(Vec<NamedField>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamedField {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub bound: Vec<PathType>,
    pub members: Vec<Member>,
    pub end: Option<String>,
}

/// `impl Первый for Второй { … }` или `impl Первый { … }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplDecl {
    pub generics: Vec<GenericParam>,
    pub first: PathType,
    pub for_type: Option<PathType>,
    pub members: Vec<Member>,
    pub end: Option<EndImpl>,
    /// Строка слова `impl`.
    pub line: u32,
}

/// Метка `end impl Первый for Второй`.
#[derive(Debug, Clone, PartialEq)]
pub struct EndImpl {
    pub first: Vec<String>,
    pub for_type: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub kind: MemberKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberKind {
    Sig(FuncSig),
    Func(FuncDecl),
    Binding(Binding),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestDecl {
    pub name: StrLit,
    pub body: Block,
    pub has_end: bool,
}

/// `extern "C" trust "обоснование" { … }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlock {
    pub abi: StrLit,
    pub reason: StrLit,
    pub members: Vec<Member>,
    pub has_end: bool,
}

// ───────── типы ─────────

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Path(PathType),
    Any(Option<PathType>),
    Ref(bool, Option<Origin>, Box<Type>),
    Ptr(bool, Box<Type>),
    Unit,
    Tuple(Vec<Type>),
    Func(Box<FuncType>),
}

/// Источник ссылки в скобках после `ref`: имя или `self`.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    Name(String),
    SelfValue,
}

/// Имя типа через точку и его аргументы в квадратных скобках.
#[derive(Debug, Clone, PartialEq)]
pub struct PathType {
    pub names: Vec<String>,
    pub args: Vec<GenericArg>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GenericArg {
    Type(Type),
    Int(String),
    Uses(Vec<ReqItem>),
    Life(LifeSource),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LifeSource {
    SelfValue,
    Names(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncType {
    pub kind: Option<FuncKind>,
    pub params: Vec<FuncTypeParam>,
    pub ret: Option<Type>,
    pub reqs: Option<Reqs>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FuncKind {
    Mut,
    Take,
    Any,
    Extern(StrLit),
}

/// Параметр в типе функции. Имя необязательно; `_` хранится как имя `_`.
#[derive(Debug, Clone, PartialEq)]
pub struct FuncTypeParam {
    pub mode: Option<Mode>,
    pub name: Option<String>,
    pub ty: Type,
}

// ───────── инструкции ─────────

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Binding(Binding),
    Expr(Expr),
    /// Присваивание; операция задана у составных: `+=` хранит `Add`.
    Assign(Option<BinOp>, Expr, Expr),
    For(ForStmt),
    While(WhileStmt),
    Loop(LoopStmt),
    Return(Option<Expr>),
    Break(Option<String>),
    Continue(Option<String>),
    Assert(Expr, Option<Expr>),
    DiscardSelf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub label: Option<String>,
    pub target: ForTarget,
    pub mode: Option<Mode>,
    pub iter: Expr,
    pub body: Block,
    pub end: Option<EndLoop>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForTarget {
    Name(String),
    Wildcard,
    Tuple(Pattern),
}

/// Метка после цикла: слово `for`, `while` или `loop` либо имя метки цикла.
#[derive(Debug, Clone, PartialEq)]
pub enum EndLoop {
    Keyword(String),
    Label(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub label: Option<String>,
    pub cond: Expr,
    pub body: Block,
    pub end: Option<EndLoop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoopStmt {
    pub label: Option<String>,
    pub body: Block,
    pub end: Option<EndLoop>,
}

// ───────── выражения ─────────

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    RangeExcl,
    RangeIncl,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    /// Написание операции в тексте программы.
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Or => "or",
            BinOp::And => "and",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::RangeExcl => "..<",
            BinOp::RangeIncl => "..=",
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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    BitNot,
    Try,
    Await,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Name(String),
    SelfValue,
    Int(String),
    Float(String),
    Str(StrLit),
    Char(String),
    Bool(bool),
    Unit,
    Paren(Box<Expr>),
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    /// Цепочка двухместных операций одного семейства, слева направо:
    /// первый операнд и пары «операция, следующий операнд».
    Binary(Box<Expr>, Vec<(BinOp, Expr)>),
    Unary(UnOp, Box<Expr>),
    /// `значение is образец`.
    Is(Box<Expr>, Pattern),
    Catch(Box<CatchExpr>),
    /// Основа и приписанное к ней по порядку: `.имя`, вызовы и индексы.
    Postfix(Box<Expr>, Vec<PostfixOp>),
    If(Box<IfExpr>),
    Match(Box<MatchExpr>),
    Closure(Box<Closure>),
    Trust(Box<TrustExpr>),
    Provide(Box<ProvideExpr>),
    Target(Box<TargetExpr>),
    LevelBlock(Box<LevelBlock>),
}

/// Строковый литерал: написание целиком и подстановки внутри него по порядку.
#[derive(Debug, Clone, PartialEq)]
pub struct StrLit {
    pub raw: String,
    pub subs: Vec<StrSub>,
    pub line: u32,
}

/// Подстановка в строке: `$имя` или `${выражение}`.
#[derive(Debug, Clone, PartialEq)]
pub enum StrSub {
    Name(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchExpr {
    pub value: Expr,
    pub name: Option<String>,
    pub body: Block,
    pub has_end: bool,
}

/// Аргумент вызова: значение, `mut значение` или `имя: значение`.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Plain(Expr),
    Mut(Expr),
    Named(String, Expr),
}

/// То, что стоит в индексных скобках: выражение, `any Тип`, `any` или `uses …`.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexArg {
    Expr(Expr),
    Any(Option<PathType>),
    Uses(Vec<ReqItem>),
}

/// Одно звено после основы выражения: поле, вызов или индекс.
#[derive(Debug, Clone, PartialEq)]
pub enum PostfixOp {
    Field(String),
    Call(Vec<Arg>),
    Index(Vec<IndexArg>),
}

/// `if … else if … else …`: ветви с условиями по порядку и необязательный блок `else`.
#[derive(Debug, Clone, PartialEq)]
pub struct IfExpr {
    pub branches: Vec<IfBranch>,
    pub otherwise: Option<Block>,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfBranch {
    pub cond: Expr,
    pub then: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchExpr {
    pub mode: Option<Mode>,
    pub value: Expr,
    pub items: Vec<MatchItem>,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchItem {
    Case(MatchCase),
    /// `else`: блок, есть ли `end else`, и строка слова `else`.
    Else(Block, bool, u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Block,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub reqs: Option<Reqs>,
    pub body: Block,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrustExpr {
    pub reason: StrLit,
    pub body: Block,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProvideExpr {
    pub items: Vec<ProvideItem>,
    pub body: Block,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProvideItem {
    pub name: String,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetExpr {
    pub items: Vec<TargetItem>,
    pub has_end: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TargetItem {
    Case(Vec<String>, Block, bool),
    Else(Block, bool),
}

/// Блок с пометкой уровня или строгости: `@low { … } end @low`.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelBlock {
    pub marker: String,
    pub body: Block,
    pub end: Option<String>,
}

// ───────── образцы ─────────

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    Wildcard,
    /// Число: признак знака «минус» и написание.
    Int(bool, String),
    Float(bool, String),
    Str(StrLit),
    Char(String),
    Bool(bool),
    Path(Vec<String>),
    Ctor(Vec<String>, Vec<PatternArg>),
    Tuple(Vec<Pattern>),
}

/// Часть образца в скобках: образец, `имя: образец` или `..`.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternArg {
    Pattern(Pattern),
    Field(String, Pattern),
    Rest,
}
