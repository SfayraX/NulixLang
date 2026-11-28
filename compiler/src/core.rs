//! Проверенное дерево явного ядра — то, что проверка исходника (`check.rs`) отдаёт переводу в IR.
//!
//! Правила — `spec/core-check.md`. Всё, что лежит здесь, уже проверено: имена разрешены, у
//! каждого выражения записан тип, простые типы отмечены, `match` полон и без недостижимых
//! ветвей, а владение верно. У каждого использования места и части временного значения
//! записано, копия это, перемещение или заимствование (`Use`); это ставит проверка владения
//! (`moves.rs`).

use crate::ir::{Mode, Scalar};

/// Номер записи или перечисления в `Program::types`.
pub type TypeId = usize;

/// Номер функции в `Program::funcs`.
pub type FuncId = usize;

/// Номер локального имени в `Body::locals`.
pub type LocalId = usize;

/// Номер цикла в теле. Циклы нумеруются в порядке проверки, см. `Body::locals`.
pub type LoopId = usize;

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
    /// У типа есть значения (9.36; `spec/core-check.md`, решение 28): у записи — если они есть
    /// у всех полей, у перечисления — если есть вариант, у всех частей которого они есть.
    pub inhabited: bool,
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
    /// Тело; его первые локальные имена — параметры, в том же порядке.
    pub body: Body,
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
    pub body: Body,
}

/// Тело функции или теста: все его локальные имена и блок.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Body {
    /// Параметры — первыми и по порядку, затем переменные и имена из образцов в порядке
    /// проверки. Он может отличаться от порядка в тексте: ветвь `if` и операнд со своим типом
    /// проверяются первыми (`spec/core-check.md`, раздел 4, «Операции»). Имена в разных блоках
    /// могут повторяться: затенения нет только среди видимых (12.93).
    pub locals: Vec<Local>,
    pub block: Block,
}

/// Локальное имя: параметр, переменная или имя из образца `match`.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub name: String,
    pub ty: Type,
    pub kind: LocalKind,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// `let`: значение не меняется (12.12).
    Let,
    Var,
    Param(Mode),
    /// Имя из образца `match`, по режиму, которым `match` сравнивает значение (`Match::mode`).
    /// Меняются только имена из `match mut` (решение 16); копия это, представление или
    /// забранная часть — раздел 5.
    Bound(Mode),
}

/// Блок: инструкции и значение. Значение — последнее выражение блока; его нет у блока типа
/// `()`: там последнее выражение стоит инструкцией (решение 4). Блок, который всегда уходит,
/// имеет тип `Never` и значения не даёт (9.36).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub value: Option<Box<Expr>>,
    pub ty: Type,
    pub line: u32,
}

impl Default for Block {
    fn default() -> Block {
        Block {
            stmts: Vec::new(),
            value: None,
            ty: Type::Unit,
            line: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `let` или `var` с одним именем.
    Let(LocalId, Expr),
    /// Разбор кортежа: `let (a, b) = …` (13.10).
    LetTuple(TuplePattern, Expr),
    /// `место = значение`; у составного присваивания — его операция: `+=` хранит `Add` (12.54).
    Assign(Place, Option<BinOp>, Expr),
    /// Выражение, стоящее инструкцией. Его значение — `()` или результат вызова функции, и оно
    /// выбрасывается (решение 4), либо выражение уходит и значения не даёт (9.36).
    Expr(Expr),
    While(LoopId, Expr, Block),
    Loop(LoopId, Block),
    Return(Option<Expr>),
    Break(LoopId),
    Continue(LoopId),
    /// `assert условие` и значение строки-сообщения.
    Assert(Expr, Option<String>),
}

/// Образец в `let (…) = …`: имя, `_` или вложенный кортеж (решение 18).
#[derive(Debug, Clone, PartialEq)]
pub enum TuplePattern {
    Bind(LocalId),
    Skip,
    Tuple(Vec<TuplePattern>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Целое число; его тип — в `Expr::ty`.
    Int(i128),
    /// Дробное число, уже округлённое до своего типа: у `f32` — прямо до ближайшего `f32`.
    Float(f64),
    Bool(bool),
    Char(char),
    Unit,
    /// Значение места: переменной, параметра, их поля или элемента, — и что с ним делается.
    Place(Place, Use),
    /// Часть значения, которое не место, — например, результата вызова: путь из полей и
    /// элементов, как у места, — и что с ней делается.
    Part(Box<Expr>, Vec<Step>, Use),
    Unary(UnOp, Box<Expr>),
    /// Цепочка двухместных операций одного семейства — первый операнд и пары «операция,
    /// операнд», как в тексте. Операции идут слева направо: `a - b + c` — это `(a - b) + c`
    /// (13.5). В арифметике и у `&`, `|`, `^` все операнды и результат — одного типа. У сравнения
    /// и сдвига операция одна: сравнение даёт `bool`, сдвиг — тип левого операнда.
    ///
    /// Цепочка не вкладывается сама в себя, поэтому глубина дерева остаётся в пределе
    /// вложенности разборщика при любой длине цепочки.
    Binary(Box<Expr>, Vec<(BinOp, Expr)>),
    /// `a and b and …`: следующая часть вычисляется, только если все прежние истинны (9.29).
    And(Vec<Expr>),
    /// `a or b or …`: следующая часть вычисляется, только если все прежние ложны (9.29).
    Or(Vec<Expr>),
    Call(FuncId, Vec<Arg>),
    /// Преобразование числа `T(x)` (12.57).
    Convert(Scalar, Box<Expr>),
    /// Запись: номер поля и значение, в порядке записи — так они и вычисляются (9.27).
    Record(TypeId, Vec<(usize, Expr)>),
    /// Вариант: тип, номер варианта и значение.
    Variant(TypeId, usize, VariantValue),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    /// `if … else if … else …`: ветви с условиями по порядку и блок `else`.
    If(Vec<(Expr, Block)>, Option<Block>),
    Match(Box<Match>),
}

/// `match` (12.41, `spec/core-check.md`, раздел 6). Он полон, и недостижимых ветвей в нём нет:
/// к каждой ветви подходит хотя бы одно значение, а `else`, если он есть, стоит последним. Без
/// ветвей бывает только `match` по значению типа без значений, и его тип — `Never` (решение 28).
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Сравниваемое значение; у `match mut` — место.
    pub value: Expr,
    /// Режим, которым сравнивается значение (12.62): `match mut` — `Mut`; `match take` и
    /// временное значение без слова — `Take`, его образец забирает сам; место без слова — `Read`.
    pub mode: Mode,
    pub cases: Vec<Case>,
    pub otherwise: Option<Block>,
}

/// Ветвь `case`: альтернативы, условие и тело. Альтернативы связывают одни и те же имена одних
/// и тех же типов (12.43), поэтому у имени во всех альтернативах один номер. При ложном условии
/// проверка идёт к следующей альтернативе, затем к следующей ветви (решение 27).
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Block,
    pub line: u32,
}

/// Образец `match` и тип значения, к которому он прикладывается.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub ty: Type,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    /// `_`: подходит любое значение.
    Skip,
    /// Имя: подходит любое значение, имя связывается с ним.
    Bind(LocalId),
    Int(i128),
    Char(char),
    Bool(bool),
    /// Вариант: перечисление, номер варианта и образцы его значения. У варианта без значения
    /// образцов нет, у безымянного значения — один, у именованных полей — по одному на поле в
    /// порядке объявления; пропущенные `_` и `..` поля — `Skip`.
    Variant(TypeId, usize, Vec<Pattern>),
    Tuple(Vec<Pattern>),
}

/// Значение, которое несёт создаваемый вариант.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantValue {
    None,
    One(Box<Expr>),
    /// Номер поля и значение, в порядке записи.
    Fields(Vec<(usize, Expr)>),
}

/// Аргумент вызова. У аргумента параметра `mut` значение — место.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub mode: Mode,
    pub value: Expr,
}

/// Что делается со значением места или части временного значения (`spec/core-check.md`,
/// раздел 5). Ставит проверка владения (`moves.rs`) по тому, куда идёт значение, и по
/// простоте его типа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Простое значение копируется (10.2).
    Copy,
    /// Непростое значение перемещается (10.1).
    Move,
    /// Непростое значение читается на месте: его отдают параметру чтения или сравнивают в
    /// `match` без слова.
    Borrow,
    /// Значение меняется на месте: его отдают параметру `mut` или сравнивают в `match mut`.
    BorrowMut,
}

/// Место: локальное имя и путь в нём.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub base: LocalId,
    pub path: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Поле записи по номеру.
    Field(usize),
    /// Элемент массива; номер — выражение типа `i64` (12.19).
    Elem(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    BitNot,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
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
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
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

    /// У типа есть значения. Их нет у `Never`, у записи или перечисления без значений
    /// (`TypeDecl::inhabited`), у кортежа с частью без значений и у непустого массива из них.
    pub fn has_values(&self, ty: &Type) -> bool {
        match ty {
            Type::Scalar(_) | Type::Unit => true,
            Type::Never => false,
            Type::Tuple(items) => items.iter().all(|item| self.has_values(item)),
            Type::Array(item, n) => *n == 0 || self.has_values(item),
            Type::Named(id) => self.types[*id].inhabited,
        }
    }

    /// У варианта с таким значением есть значения: они есть у всех его частей (решение 28).
    pub fn payload_has_values(&self, payload: &Payload) -> bool {
        match payload {
            Payload::None => true,
            Payload::One(ty) => self.has_values(ty),
            Payload::Fields(fields) => fields.iter().all(|f| self.has_values(&f.ty)),
        }
    }

    /// Перечисление без вариантов — `Never` или своё (9.36): вызов функции с таким результатом
    /// уходит (решение 10в). Значений нет и у других типов — см. `has_values`.
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
