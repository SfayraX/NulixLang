//! Переносимый IR ядра: данные.
//!
//! Устройство описано в `spec/ir-core.md`; текстовая форма — в `ir_text.rs`. Здесь то, что нужно
//! шагу 2: явное ядро без ссылок как значений, обобщений, требований и расширений.
//!
//! Тело функции — инструкции в порядке выполнения; ветвления и циклы — вложенные блоки.
//! Всё выведенное записано: тип каждой переменной, способ каждого использования (копия,
//! перемещение, заимствование), уничтожения и состояние владения в точках слияния.
//! Имена переменных хранятся для чтения: в хеш и двоичную запись они не войдут.
//!
//! Имена непусты и не содержат обратной кавычки и управляющих символов. Имя функции может быть
//! составным, `Point.norm`: части разделены точкой и сами точек не содержат.

/// Версия IR, которую читает и пишет эта реализация.
pub const VERSION: u32 = 1;

/// Модуль: версия IR и объявления по порядку.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub version: u32,
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Struct(StructDecl),
    Enum(EnumDecl),
    Func(Func),
    Test(Test),
}

/// Запись. `copy` — тип простой: его значения копируются, а не перемещаются.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub copy: bool,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
}

/// Перечисление с вариантами. Перечисление без вариантов, например `Never`, значений не имеет.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: String,
    pub copy: bool,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub payload: Payload,
}

/// Что несёт вариант: ничего, одно безымянное значение или именованные поля — не меньше одного.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    None,
    One(Type),
    Fields(Vec<Field>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Func {
    pub name: String,
    pub params: Vec<Param>,
    /// Тип результата; у функции без результата — `Type::Unit`.
    pub result: Type,
    /// Требования `uses`. На шаге 2 всегда пусты.
    pub uses: Vec<String>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub mode: Mode,
}

/// Режим параметра: только чтение, изменение на месте или передача владения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Read,
    Mut,
    Take,
}

/// Тест `test "имя" { … }`: функция без параметров и результата.
#[derive(Debug, Clone, PartialEq)]
pub struct Test {
    pub name: String,
    pub body: Block,
}

/// Встроенные типы-скаляры.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Char,
}

pub const SCALARS: [Scalar; 12] = [
    Scalar::I8,
    Scalar::I16,
    Scalar::I32,
    Scalar::I64,
    Scalar::U8,
    Scalar::U16,
    Scalar::U32,
    Scalar::U64,
    Scalar::F32,
    Scalar::F64,
    Scalar::Bool,
    Scalar::Char,
];

impl Scalar {
    pub fn name(self) -> &'static str {
        match self {
            Scalar::I8 => "i8",
            Scalar::I16 => "i16",
            Scalar::I32 => "i32",
            Scalar::I64 => "i64",
            Scalar::U8 => "u8",
            Scalar::U16 => "u16",
            Scalar::U32 => "u32",
            Scalar::U64 => "u64",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
            Scalar::Bool => "bool",
            Scalar::Char => "char",
        }
    }

    pub fn from_name(name: &str) -> Option<Scalar> {
        SCALARS.iter().copied().find(|s| s.name() == name)
    }

    /// Пределы целого типа; у остальных скаляров их нет.
    pub fn int_range(self) -> Option<(i128, i128)> {
        Some(match self {
            Scalar::I8 => (i8::MIN as i128, i8::MAX as i128),
            Scalar::I16 => (i16::MIN as i128, i16::MAX as i128),
            Scalar::I32 => (i32::MIN as i128, i32::MAX as i128),
            Scalar::I64 => (i64::MIN as i128, i64::MAX as i128),
            Scalar::U8 => (0, u8::MAX as i128),
            Scalar::U16 => (0, u16::MAX as i128),
            Scalar::U32 => (0, u32::MAX as i128),
            Scalar::U64 => (0, u64::MAX as i128),
            _ => return None,
        })
    }

    /// Знаковое целое.
    pub fn is_signed(self) -> bool {
        matches!(self, Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::I64)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Scalar(Scalar),
    /// Единичный тип `()`.
    Unit,
    /// Кортеж, не меньше двух элементов.
    Tuple(Vec<Type>),
    /// Массив `Array[T, N]`.
    Array(Box<Type>, u64),
    /// Запись или перечисление, объявленные в модуле.
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `local x: T` — переменная без значения; значение даст присваивание.
    Local {
        name: String,
        ty: Type,
    },
    /// `x: T = …` — новая переменная со значением.
    Define {
        name: String,
        ty: Type,
        value: Rhs,
    },
    /// `место = …` — значение в уже объявленное место.
    Assign {
        place: Place,
        value: Rhs,
    },
    /// `drop место` — уничтожение значения.
    Drop(Place),
    /// `swap место, место` — обмен значений двух мест одного типа; ни одно место не пустеет.
    Swap(Place, Place),
    /// `call f(…)` — вызов, результат которого единичный.
    Call {
        func: String,
        args: Vec<Operand>,
    },
    /// `if x { … } else { … }`; `after` — состояние после слияния, если конец достижим.
    If {
        cond: Place,
        then: Block,
        otherwise: Block,
        after: Option<State>,
    },
    /// `switch место { case … }` по варианту или по значению скаляра.
    Switch {
        on: Place,
        cases: Vec<Case>,
        otherwise: Option<Block>,
        after: Option<State>,
    },
    /// `loop L [held: …] { … }`: `head` — состояние в начале каждого прохода,
    /// `after` — после выхода из цикла, если выход есть.
    Loop {
        label: String,
        head: State,
        body: Block,
        after: Option<State>,
    },
    Break(String),
    Continue(String),
    Return(Option<Operand>),
    /// `crash "сообщение"` — авария.
    Crash(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub label: CaseLabel,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaseLabel {
    /// Вариант перечисления.
    Variant(String),
    /// Значение скаляра: целое, символ или логическое.
    Value(Const),
}

/// Правая часть: откуда берётся значение.
#[derive(Debug, Clone, PartialEq)]
pub enum Rhs {
    Use(Operand),
    /// Операция с проверкой: `add.i64(…)`; операндов столько, сколько даёт `Op::arity`.
    Op {
        op: Op,
        ty: Scalar,
        args: Vec<Operand>,
    },
    /// Преобразование числа: `conv.i64.u8(…)`; если значение не помещается — авария.
    Conv {
        from: Scalar,
        to: Scalar,
        arg: Operand,
    },
    Call {
        func: String,
        args: Vec<Operand>,
    },
    /// Запись — `make T(поля)`, скобки есть всегда, даже пустые; вариант — `make T.V`,
    /// `make T.V(значение)` или `make T.V(поля)`.
    Make {
        ty: String,
        variant: Option<String>,
        args: Option<Vec<Arg>>,
    },
    /// Кортеж, не меньше двух элементов.
    Tuple(Vec<Operand>),
    Array(Vec<Operand>),
}

/// Аргумент `make`: с именем поля или без.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Operand,
}

/// Использование значения: каждое записано явно.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    /// Заимствование для чтения; живёт до конца инструкции, в которой стоит.
    Borrow(Place),
    /// Заимствование для изменения.
    BorrowMut(Place),
    Const(Const),
}

/// Место: переменная и путь внутрь неё — поле, номер в кортеже или варианте, элемент массива.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub base: String,
    pub path: Vec<Seg>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    /// `.имя` — поле записи, вариант перечисления или поле варианта.
    Field(String),
    /// `.0` — элемент кортежа или безымянное значение варианта.
    Pos(u32),
    /// `[i]` — элемент массива.
    Elem(Index),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Index {
    Var(String),
    Lit(u64),
}

/// Состояние владения в точке слияния: места, в которых есть значение.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub held: Vec<Place>,
}

/// Константа. Её тип задаёт место, где она стоит.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(i128),
    /// Число с плавающей точкой — биты f64; только конечные значения.
    Float(u64),
    Bool(bool),
    Char(char),
    Unit,
}

/// Операции с проверкой.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    And,
    Or,
    Xor,
    Not,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

pub const OPS: [Op; 18] = [
    Op::Add,
    Op::Sub,
    Op::Mul,
    Op::Div,
    Op::Rem,
    Op::Neg,
    Op::And,
    Op::Or,
    Op::Xor,
    Op::Not,
    Op::Shl,
    Op::Shr,
    Op::Eq,
    Op::Ne,
    Op::Lt,
    Op::Le,
    Op::Gt,
    Op::Ge,
];

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Add => "add",
            Op::Sub => "sub",
            Op::Mul => "mul",
            Op::Div => "div",
            Op::Rem => "rem",
            Op::Neg => "neg",
            Op::And => "and",
            Op::Or => "or",
            Op::Xor => "xor",
            Op::Not => "not",
            Op::Shl => "shl",
            Op::Shr => "shr",
            Op::Eq => "eq",
            Op::Ne => "ne",
            Op::Lt => "lt",
            Op::Le => "le",
            Op::Gt => "gt",
            Op::Ge => "ge",
        }
    }

    pub fn from_name(name: &str) -> Option<Op> {
        OPS.iter().copied().find(|o| o.name() == name)
    }

    /// Число операндов: один у `neg` и `not`, два у остальных.
    pub fn arity(self) -> usize {
        match self {
            Op::Neg | Op::Not => 1,
            _ => 2,
        }
    }
}
