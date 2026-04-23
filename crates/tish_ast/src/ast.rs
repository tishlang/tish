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

/// Function parameter with optional type annotation and default value.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedParam {
    pub name: Arc<str>,
    pub name_span: Span,
    pub type_ann: Option<TypeAnnotation>,
    pub default: Option<Expr>,
}

/// Single formal parameter: simple identifier or destructuring pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum FunParam {
    Simple(TypedParam),
    Destructure {
        pattern: DestructPattern,
        type_ann: Option<TypeAnnotation>,
        default: Option<Expr>,
    },
}

impl FunParam {
    /// Variable names introduced by this formal parameter.
    pub fn bound_names(&self) -> Vec<Arc<str>> {
        let mut out = Vec::new();
        match self {
            FunParam::Simple(tp) => out.push(Arc::clone(&tp.name)),
            FunParam::Destructure { pattern, .. } => {
                Self::collect_pattern_binding_names(pattern, &mut out);
            }
        }
        out
    }

    fn collect_pattern_binding_names(pattern: &DestructPattern, out: &mut Vec<Arc<str>>) {
        match pattern {
            DestructPattern::Array(elements) => {
                for el in elements {
                    if let Some(el) = el {
                        match el {
                            DestructElement::Ident(n, _) => out.push(Arc::clone(n)),
                            DestructElement::Pattern(p) => {
                                Self::collect_pattern_binding_names(p, out);
                            }
                            DestructElement::Rest(n, _) => out.push(Arc::clone(n)),
                        }
                    }
                }
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    match &prop.value {
                        DestructElement::Ident(n, _) => out.push(Arc::clone(n)),
                        DestructElement::Pattern(p) => {
                            Self::collect_pattern_binding_names(p, out);
                        }
                        DestructElement::Rest(n, _) => out.push(Arc::clone(n)),
                    }
                }
            }
        }
    }
}

/// Destructuring pattern for array or object destructuring
#[derive(Debug, Clone, PartialEq)]
pub enum DestructPattern {
    /// Array destructuring: [a, b, c] or [a, , c]
    Array(Vec<Option<DestructElement>>),
    /// Object destructuring: { a, b: renamed, c }
    Object(Vec<DestructProp>),
}

/// Element in array destructuring pattern
#[derive(Debug, Clone, PartialEq)]
pub enum DestructElement {
    /// Simple binding: a
    Ident(Arc<str>, Span),
    /// Nested pattern: [a, b] or { x, y }
    Pattern(Box<DestructPattern>),
    /// Rest element: ...rest
    Rest(Arc<str>, Span),
}

/// Property in object destructuring pattern
#[derive(Debug, Clone, PartialEq)]
pub struct DestructProp {
    /// Original property name in source object
    pub key: Arc<str>,
    /// Binding name (may be same as key or renamed)
    pub value: DestructElement,
}

/// Import specifier: named (a, b: c), namespace (* as M), or default (X)
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSpecifier {
    /// Named: { foo } or { foo as bar }
    Named {
        name: Arc<str>,
        name_span: Span,
        alias: Option<Arc<str>>,
        alias_span: Option<Span>,
    },
    /// Namespace: * as M
    Namespace {
        name: Arc<str>,
        name_span: Span,
    },
    /// Default: import X from "..."
    Default {
        name: Arc<str>,
        name_span: Span,
    },
}

/// Export declaration: named (const/let/fn) or default
#[derive(Debug, Clone, PartialEq)]
pub enum ExportDeclaration {
    /// export const x = 1 / export let x / export fn f() {}
    Named(Box<Statement>),
    /// export default expr
    Default(Expr),
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Block {
        statements: Vec<Statement>,
        span: Span,
    },
    VarDecl {
        name: Arc<str>,
        name_span: Span,
        mutable: bool, // true for `let`, false for `const`
        type_ann: Option<TypeAnnotation>,
        init: Option<Expr>,
        span: Span,
    },
    /// Variable declaration with destructuring pattern
    VarDeclDestructure {
        pattern: DestructPattern,
        mutable: bool,
        init: Expr,
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
        name_span: Span,
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
        async_: bool,
        name: Arc<str>,
        name_span: Span,
        params: Vec<FunParam>,
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
        catch_param_span: Option<Span>,
        catch_body: Option<Box<Statement>>,
        finally_body: Option<Box<Statement>>,
        span: Span,
    },
    Import {
        specifiers: Vec<ImportSpecifier>,
        from: Arc<str>,
        span: Span,
    },
    Export {
        declaration: Box<ExportDeclaration>,
        span: Span,
    },
    /// `type Name = Type` (erased at runtime; for checker / declaration files).
    TypeAlias {
        name: Arc<str>,
        name_span: Span,
        ty: TypeAnnotation,
        span: Span,
    },
    /// `declare let name: T` or `declare const name: T`
    DeclareVar {
        name: Arc<str>,
        name_span: Span,
        type_ann: Option<TypeAnnotation>,
        const_: bool,
        span: Span,
    },
    /// `declare [async] function name(...): R` (no body).
    DeclareFun {
        async_: bool,
        name: Arc<str>,
        name_span: Span,
        params: Vec<FunParam>,
        rest_param: Option<TypedParam>,
        return_type: Option<TypeAnnotation>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
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
        args: Vec<CallArg>,
        span: Span,
    },
    /// `new` expression (JavaScript target). `callee` is the constructor reference; `args` may be empty.
    New {
        callee: Box<Expr>,
        args: Vec<CallArg>,
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
        elements: Vec<ArrayElement>,
        span: Span,
    },
    Object {
        props: Vec<ObjectProp>,
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
    LogicalAssign {
        name: Arc<str>,
        op: LogicalAssignOp,
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
    /// Arrow function: (params) => body
    ArrowFunction {
        params: Vec<FunParam>,
        body: ArrowBody,
        span: Span,
    },
    /// Template literal: `text ${expr} text`
    TemplateLiteral {
        quasis: Vec<Arc<str>>, // Static string parts (n+1 for n expressions)
        exprs: Vec<Expr>,      // Interpolated expressions (n)
        span: Span,
    },
    /// Await expression: await operand
    Await {
        operand: Box<Expr>,
        span: Span,
    },
    /// JSX element: <Tag props>children</Tag>
    JsxElement {
        tag: Arc<str>,
        props: Vec<JsxProp>,
        children: Vec<JsxChild>,
        span: Span,
    },
    /// JSX fragment: <>children</>
    JsxFragment {
        children: Vec<JsxChild>,
        span: Span,
    },
    /// Native module load: import { x } from 'tish:egui' → loads from tishlang_runtime
    NativeModuleLoad {
        spec: Arc<str>,
        export_name: Arc<str>,
        span: Span,
    },
}

/// JSX attribute/prop
#[derive(Debug, Clone, PartialEq)]
pub enum JsxProp {
    /// name="value" or name={expr} or name (boolean shorthand)
    Attr { name: Arc<str>, value: JsxAttrValue },
    /// {...expr}
    Spread(Expr),
}

/// JSX attribute value
#[derive(Debug, Clone, PartialEq)]
pub enum JsxAttrValue {
    /// "literal string"
    String(Arc<str>),
    /// {expr}
    Expr(Expr),
    /// name without value (e.g. disabled) = true
    ImplicitTrue,
}

/// JSX child node
#[derive(Debug, Clone, PartialEq)]
pub enum JsxChild {
    /// Text content
    Text(Arc<str>),
    /// {expr} or nested element
    Expr(Expr),
}

impl Expr {
    /// Return the source span for this expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Literal { span, .. } => *span,
            Expr::Ident { span, .. } => *span,
            Expr::Binary { span, .. } => *span,
            Expr::Unary { span, .. } => *span,
            Expr::Call { span, .. } => *span,
            Expr::New { span, .. } => *span,
            Expr::Member { span, .. } => *span,
            Expr::Index { span, .. } => *span,
            Expr::Conditional { span, .. } => *span,
            Expr::NullishCoalesce { span, .. } => *span,
            Expr::Array { span, .. } => *span,
            Expr::Object { span, .. } => *span,
            Expr::Assign { span, .. } => *span,
            Expr::TypeOf { span, .. } => *span,
            Expr::PostfixInc { span, .. } => *span,
            Expr::PostfixDec { span, .. } => *span,
            Expr::PrefixInc { span, .. } => *span,
            Expr::PrefixDec { span, .. } => *span,
            Expr::CompoundAssign { span, .. } => *span,
            Expr::LogicalAssign { span, .. } => *span,
            Expr::MemberAssign { span, .. } => *span,
            Expr::IndexAssign { span, .. } => *span,
            Expr::ArrowFunction { span, .. } => *span,
            Expr::TemplateLiteral { span, .. } => *span,
            Expr::Await { span, .. } => *span,
            Expr::JsxElement { span, .. } => *span,
            Expr::JsxFragment { span, .. } => *span,
            Expr::NativeModuleLoad { span, .. } => *span,
        }
    }
}

/// Body of an arrow function: either an expression or a block
#[derive(Debug, Clone, PartialEq)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Box<Statement>),
}

/// Array element: either a regular expression or spread element
#[derive(Debug, Clone, PartialEq)]
pub enum ArrayElement {
    Expr(Expr),
    Spread(Expr),
}

/// Object property: either a regular key-value pair or spread
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectProp {
    KeyValue(Arc<str>, Expr),
    Spread(Expr),
}

/// Function call argument: either a regular argument or spread
#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Expr(Expr),
    Spread(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    Add, // +=
    Sub, // -=
    Mul, // *=
    Div, // /=
    Mod, // %=
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalAssignOp {
    AndAnd,  // &&=
    OrOr,    // ||=
    Nullish, // ??=
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

#[derive(Debug, Clone, PartialEq)]
pub enum MemberProp {
    /// Property name in `obj.prop` / `obj?.prop` (span covers **prop** only).
    Name {
        name: Arc<str>,
        span: Span,
    },
    Expr(Box<Expr>), // for computed property
}

impl Statement {
    /// Source span covering this statement (including nested bodies where applicable).
    pub fn span(&self) -> Span {
        match self {
            Statement::Block { span, .. }
            | Statement::VarDecl { span, .. }
            | Statement::VarDeclDestructure { span, .. }
            | Statement::ExprStmt { span, .. }
            | Statement::If { span, .. }
            | Statement::While { span, .. }
            | Statement::For { span, .. }
            | Statement::ForOf { span, .. }
            | Statement::Return { span, .. }
            | Statement::Break { span, .. }
            | Statement::Continue { span, .. }
            | Statement::FunDecl { span, .. }
            | Statement::Switch { span, .. }
            | Statement::DoWhile { span, .. }
            | Statement::Throw { span, .. }
            | Statement::Try { span, .. }
            |             Statement::Import { span, .. }
            | Statement::Export { span, .. }
            | Statement::TypeAlias { span, .. }
            | Statement::DeclareVar { span, .. }
            | Statement::DeclareFun { span, .. } => *span,
        }
    }
}
