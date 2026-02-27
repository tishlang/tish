//! Abstract syntax tree for Tish.

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: (usize, usize), // line, col
    pub end: (usize, usize),
}

/// Type annotation for variables, parameters, and return types.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    /// Primitive types: number, string, boolean, null
    Simple(Arc<str>),
    /// Array type: T[]
    Array(Box<TypeAnnotation>),
    /// Object type: { key: Type, ... }
    Object(Vec<(Arc<str>, TypeAnnotation)>),
    /// Function type: (T1, T2) => R
    Function {
        params: Vec<TypeAnnotation>,
        returns: Box<TypeAnnotation>,
    },
    /// Union type: T1 | T2
    Union(Vec<TypeAnnotation>),
}

/// Function parameter with optional type annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedParam {
    pub name: Arc<str>,
    pub type_ann: Option<TypeAnnotation>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub enum Statement {
    Block {
        statements: Vec<Statement>,
        span: Span,
    },
    VarDecl {
        name: Arc<str>,
        mutable: bool, // true for `let`, false for `const`
        type_ann: Option<TypeAnnotation>,
        init: Option<Expr>,
        span: Span,
    },
    ExprStmt {
        expr: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_branch: Box<Statement>,
        else_branch: Option<Box<Statement>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Box<Statement>,
        span: Span,
    },
    For {
        init: Option<Box<Statement>>,
        cond: Option<Expr>,
        update: Option<Expr>,
        body: Box<Statement>,
        span: Span,
    },
    ForOf {
        name: Arc<str>,
        iterable: Expr,
        body: Box<Statement>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    FunDecl {
        name: Arc<str>,
        params: Vec<TypedParam>,
        rest_param: Option<TypedParam>,
        return_type: Option<TypeAnnotation>,
        body: Box<Statement>,
        span: Span,
    },
    Switch {
        expr: Expr,
        cases: Vec<(Option<Expr>, Vec<Statement>)>,
        default_body: Option<Vec<Statement>>,
        span: Span,
    },
    DoWhile {
        body: Box<Statement>,
        cond: Expr,
        span: Span,
    },
    Throw {
        value: Expr,
        span: Span,
    },
    Try {
        body: Box<Statement>,
        catch_param: Option<Arc<str>>,
        catch_body: Box<Statement>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub enum Expr {
    Literal {
        value: Literal,
        span: Span,
    },
    Ident {
        name: Arc<str>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    Member {
        object: Box<Expr>,
        prop: MemberProp,
        optional: bool,
        span: Span,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        optional: bool,
        span: Span,
    },
    Conditional {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
        span: Span,
    },
    NullishCoalesce {
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Array {
        elements: Vec<Expr>,
        span: Span,
    },
    Object {
        props: Vec<(Arc<str>, Expr)>,
        span: Span,
    },
    Assign {
        name: Arc<str>,
        value: Box<Expr>,
        span: Span,
    },
    TypeOf {
        operand: Box<Expr>,
        span: Span,
    },
    PostfixInc {
        name: Arc<str>,
        span: Span,
    },
    PostfixDec {
        name: Arc<str>,
        span: Span,
    },
    PrefixInc {
        name: Arc<str>,
        span: Span,
    },
    PrefixDec {
        name: Arc<str>,
        span: Span,
    },
    CompoundAssign {
        name: Arc<str>,
        op: CompoundOp,
        value: Box<Expr>,
        span: Span,
    },
    /// Property assignment: obj.prop = value
    MemberAssign {
        object: Box<Expr>,
        prop: Arc<str>,
        value: Box<Expr>,
        span: Span,
    },
    /// Index assignment: arr[index] = value
    IndexAssign {
        object: Box<Expr>,
        index: Box<Expr>,
        value: Box<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    Add,  // +=
    Sub,  // -=
    Mul,  // *=
    Div,  // /=
    Mod,  // %=
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    Pos,
    BitNot,
    Void,
}

#[derive(Debug, Clone)]
pub enum MemberProp {
    Name(Arc<str>),
    Expr(Box<Expr>), // for computed property
}
