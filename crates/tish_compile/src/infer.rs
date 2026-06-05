//! Type inference pass: annotates `VarDecl` nodes with inferred `TypeAnnotation`s
//! where the user hasn't provided them, enabling codegen to emit native Rust types.
//!
//! Rules (conservative — only infer when unambiguous):
//!   - Number literal init           → `number`
//!   - String literal init           → `string`
//!   - Bool literal init             → `boolean`
//!   - Arithmetic of two `number` expressions → `number`
//!   - Comparison of two `number` expressions → `boolean`
//!   - Already-annotated vars are left unchanged.

use std::collections::HashMap;
use tishlang_ast::{
    ArrowBody, BinOp, CallArg, Expr, FunParam, Literal, Program, Statement, TypeAnnotation,
};

/// Scoped type environment used during inference.
#[derive(Default)]
pub struct InferCtx {
    scopes: Vec<HashMap<String, TypeAnnotation>>,
}

impl InferCtx {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: &str, ty: TypeAnnotation) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), ty);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&TypeAnnotation> {
        for s in self.scopes.iter().rev() {
            if let Some(t) = s.get(name) {
                return Some(t);
            }
        }
        None
    }
}

fn is_number(ann: &TypeAnnotation) -> bool {
    matches!(ann, TypeAnnotation::Simple(s) if s.as_ref() == "number")
}

fn number_ann() -> TypeAnnotation {
    TypeAnnotation::Simple("number".into())
}

fn string_ann() -> TypeAnnotation {
    TypeAnnotation::Simple("string".into())
}

fn bool_ann() -> TypeAnnotation {
    TypeAnnotation::Simple("boolean".into())
}

/// Infer the `TypeAnnotation` for an expression, if unambiguous.
pub fn infer_expr_type(expr: &Expr, ctx: &InferCtx) -> Option<TypeAnnotation> {
    match expr {
        Expr::Literal { value, .. } => match value {
            Literal::Number(_) => Some(number_ann()),
            Literal::String(_) => Some(string_ann()),
            Literal::Bool(_) => Some(bool_ann()),
            Literal::Null => None,
        },
        Expr::Ident { name, .. } => ctx.lookup(name.as_ref()).cloned(),
        Expr::Binary {
            left, op, right, ..
        } => {
            let lt = infer_expr_type(left, ctx)?;
            let rt = infer_expr_type(right, ctx)?;
            if is_number(&lt) && is_number(&rt) {
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                        Some(number_ann())
                    }
                    BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::StrictEq
                    | BinOp::StrictNe => Some(bool_ann()),
                    _ => None,
                }
            } else {
                None
            }
        }
        Expr::Unary { op, operand, .. } => {
            use tishlang_ast::UnaryOp;
            match op {
                UnaryOp::Neg | UnaryOp::Pos => {
                    let t = infer_expr_type(operand, ctx)?;
                    if is_number(&t) {
                        Some(number_ann())
                    } else {
                        None
                    }
                }
                UnaryOp::Not => Some(bool_ann()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Run inference over a program, returning a modified Program with additional
/// type annotations filled in on `VarDecl` nodes.
pub fn infer_program(program: &Program) -> Program {
    let mut ctx = InferCtx::new();
    let p = Program {
        statements: infer_statements(&program.statements, &mut ctx),
    };
    // Automatic struct inference (opt-in via TISH_STRUCT_INFER until proven):
    // give unannotated object literals a concrete struct type so the Rust
    // backend emits unboxed structs with direct field access. Conservative —
    // only applies when every use of the binding is a literal-key field read,
    // so it can never miscompile (any uncertainty falls back to boxed Value).
    if std::env::var("TISH_STRUCT_INFER").map(|v| v != "0").unwrap_or(false) {
        struct_infer_program(p)
    } else {
        p
    }
}

// ---------------------------------------------------------------------------
// Automatic struct inference (conservative, sound, opt-in)
// ---------------------------------------------------------------------------

/// Registry of distinct inferred object shapes → synthetic alias name, so
/// identical shapes share one generated struct.
#[derive(Default)]
struct StructRegistry {
    /// canonical "k1:ty1;k2:ty2;…" → alias name
    by_shape: HashMap<String, String>,
    /// alias name → field list (for emitting the `type` decls)
    decls: Vec<(String, Vec<(std::sync::Arc<str>, TypeAnnotation)>)>,
}

impl StructRegistry {
    fn intern(&mut self, fields: &[(std::sync::Arc<str>, TypeAnnotation)]) -> String {
        let canon = fields
            .iter()
            .map(|(k, t)| format!("{}:{}", k, type_canon(t)))
            .collect::<Vec<_>>()
            .join(";");
        if let Some(name) = self.by_shape.get(&canon) {
            return name.clone();
        }
        let name = format!("TishAnon_{}", self.decls.len());
        self.by_shape.insert(canon, name.clone());
        self.decls.push((name.clone(), fields.to_vec()));
        name
    }
}

fn type_canon(t: &TypeAnnotation) -> String {
    match t {
        TypeAnnotation::Simple(s) => s.to_string(),
        TypeAnnotation::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(k, t)| format!("{}:{}", k, type_canon(t)))
                .collect::<Vec<_>>()
                .join(";")
        ),
        _ => "?".to_string(),
    }
}

/// Infer a concrete object shape from an object literal, or `None` if any field
/// can't be typed concretely / there's a spread.
fn infer_object_shape(
    props: &[tishlang_ast::ObjectProp],
    ctx: &InferCtx,
) -> Option<Vec<(std::sync::Arc<str>, TypeAnnotation)>> {
    let mut fields = Vec::with_capacity(props.len());
    for p in props {
        match p {
            tishlang_ast::ObjectProp::KeyValue(k, v) => {
                let ty = infer_expr_type(v, ctx)?;
                // Only primitive field types in this conservative version.
                if !matches!(&ty, TypeAnnotation::Simple(s)
                    if matches!(s.as_ref(), "number" | "string" | "boolean"))
                {
                    return None;
                }
                fields.push((k.clone(), ty));
            }
            tishlang_ast::ObjectProp::Spread(_) => return None,
        }
    }
    if fields.is_empty() {
        return None;
    }
    Some(fields)
}

fn struct_infer_program(program: Program) -> Program {
    let mut reg = StructRegistry::default();
    let mut ctx = InferCtx::new();
    let mut stmts = si_block(program.statements, &mut reg, &mut ctx);
    // Prepend the generated struct `type` aliases so codegen synthesizes them.
    let mut out: Vec<Statement> = Vec::with_capacity(stmts.len() + reg.decls.len());
    let span = stmts.first().map(stmt_span).unwrap_or_else(zero_span);
    for (name, fields) in reg.decls.drain(..) {
        out.push(Statement::TypeAlias {
            name: name.as_str().into(),
            name_span: span,
            ty: TypeAnnotation::Object(fields),
            span,
        });
    }
    out.append(&mut stmts);
    Program { statements: out }
}

fn stmt_span(s: &Statement) -> tishlang_ast::Span {
    match s {
        Statement::VarDecl { span, .. }
        | Statement::Block { span, .. }
        | Statement::ExprStmt { span, .. }
        | Statement::If { span, .. }
        | Statement::For { span, .. }
        | Statement::ForOf { span, .. }
        | Statement::While { span, .. }
        | Statement::Return { span, .. }
        | Statement::FunDecl { span, .. }
        | Statement::TypeAlias { span, .. } => *span,
        _ => zero_span(),
    }
}

fn zero_span() -> tishlang_ast::Span {
    tishlang_ast::Span {
        start: (0, 0),
        end: (0, 0),
    }
}

/// Transform a block: annotate struct-safe object `let` bindings, recursing
/// into nested blocks. `ctx` provides field-type inference for initializers.
fn si_block(stmts: Vec<Statement>, reg: &mut StructRegistry, ctx: &mut InferCtx) -> Vec<Statement> {
    ctx.push_scope();
    let n = stmts.len();
    let mut out: Vec<Statement> = Vec::with_capacity(n);
    for (i, stmt) in stmts.iter().enumerate() {
        // Candidate: `let o = { ...object literal... }` with no annotation.
        if let Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann: None,
            init: Some(Expr::Object { props, .. }),
            span,
        } = stmt
        {
            if let Some(fields) = infer_object_shape(props, ctx) {
                let keys: std::collections::HashSet<&str> =
                    fields.iter().map(|(k, _)| k.as_ref()).collect();
                // Sound: every later use in this block must be a literal-key read.
                if uses_are_struct_safe(name.as_ref(), &keys, &stmts[i + 1..]) {
                    let alias = reg.intern(&fields);
                    ctx.define(name.as_ref(), TypeAnnotation::Simple(alias.as_str().into()));
                    out.push(Statement::VarDecl {
                        name: name.clone(),
                        name_span: *name_span,
                        mutable: *mutable,
                        type_ann: Some(TypeAnnotation::Simple(alias.as_str().into())),
                        init: stmt_init_clone(stmt),
                        span: *span,
                    });
                    continue;
                }
            }
        }
        out.push(si_recurse(stmt, reg, ctx));
    }
    ctx.pop_scope();
    out
}

fn stmt_init_clone(stmt: &Statement) -> Option<Expr> {
    if let Statement::VarDecl { init, .. } = stmt {
        init.clone()
    } else {
        None
    }
}

/// Recurse struct inference into a statement's nested blocks (function bodies,
/// loop/if bodies). Non-block statements pass through unchanged.
fn si_recurse(stmt: &Statement, reg: &mut StructRegistry, ctx: &mut InferCtx) -> Statement {
    match stmt {
        Statement::Block { statements, span } => Statement::Block {
            statements: si_block(statements.clone(), reg, ctx),
            span: *span,
        },
        Statement::For {
            init,
            cond,
            update,
            body,
            span,
        } => Statement::For {
            init: init.clone(),
            cond: cond.clone(),
            update: update.clone(),
            body: Box::new(si_recurse(body, reg, ctx)),
            span: *span,
        },
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            span,
        } => Statement::ForOf {
            name: name.clone(),
            name_span: *name_span,
            iterable: iterable.clone(),
            body: Box::new(si_recurse(body, reg, ctx)),
            span: *span,
        },
        Statement::While { cond, body, span } => Statement::While {
            cond: cond.clone(),
            body: Box::new(si_recurse(body, reg, ctx)),
            span: *span,
        },
        Statement::DoWhile { body, cond, span } => Statement::DoWhile {
            body: Box::new(si_recurse(body, reg, ctx)),
            cond: cond.clone(),
            span: *span,
        },
        Statement::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => Statement::If {
            cond: cond.clone(),
            then_branch: Box::new(si_recurse(then_branch, reg, ctx)),
            else_branch: else_branch.as_ref().map(|e| Box::new(si_recurse(e, reg, ctx))),
            span: *span,
        },
        Statement::FunDecl {
            async_,
            name,
            name_span,
            params,
            rest_param,
            return_type,
            body,
            span,
        } => Statement::FunDecl {
            async_: *async_,
            name: name.clone(),
            name_span: *name_span,
            params: params.clone(),
            rest_param: rest_param.clone(),
            return_type: return_type.clone(),
            body: Box::new(si_recurse(body, reg, ctx)),
            span: *span,
        },
        other => other.clone(),
    }
}

/// Sound check: in `tail`, every use of `name` is a literal-key field READ
/// (`name.field` with field ∈ `keys`). Any other occurrence — write, computed
/// access, reassignment, escape into a call/return/array/object/closure, or a
/// rebinding of `name` — returns false (bail to boxed Value). Unhandled AST
/// shapes also return false, so this can never wrongly green-light.
fn uses_are_struct_safe(name: &str, keys: &std::collections::HashSet<&str>, tail: &[Statement]) -> bool {
    tail.iter().all(|s| stmt_name_safe(s, name, keys))
}

fn opt_expr_safe(e: &Option<Expr>, name: &str, keys: &std::collections::HashSet<&str>) -> bool {
    e.as_ref().map(|e| expr_name_safe(e, name, keys)).unwrap_or(true)
}

fn stmt_name_safe(s: &Statement, name: &str, keys: &std::collections::HashSet<&str>) -> bool {
    match s {
        // A rebinding of `name` in scope is too subtle to track — bail.
        Statement::VarDecl { name: n, init, .. } => {
            if n.as_ref() == name {
                return false;
            }
            opt_expr_safe(init, name, keys)
        }
        Statement::VarDeclDestructure { init, .. } => expr_name_safe(init, name, keys),
        Statement::ExprStmt { expr, .. } => expr_name_safe(expr, name, keys),
        Statement::Block { statements, .. } => statements.iter().all(|s| stmt_name_safe(s, name, keys)),
        Statement::If { cond, then_branch, else_branch, .. } => {
            expr_name_safe(cond, name, keys)
                && stmt_name_safe(then_branch, name, keys)
                && else_branch.as_ref().map(|e| stmt_name_safe(e, name, keys)).unwrap_or(true)
        }
        Statement::While { cond, body, .. } => {
            expr_name_safe(cond, name, keys) && stmt_name_safe(body, name, keys)
        }
        Statement::DoWhile { body, cond, .. } => {
            stmt_name_safe(body, name, keys) && expr_name_safe(cond, name, keys)
        }
        Statement::For { init, cond, update, body, .. } => {
            init.as_ref().map(|i| stmt_name_safe(i, name, keys)).unwrap_or(true)
                && cond.as_ref().map(|c| expr_name_safe(c, name, keys)).unwrap_or(true)
                && update.as_ref().map(|u| expr_name_safe(u, name, keys)).unwrap_or(true)
                && stmt_name_safe(body, name, keys)
        }
        Statement::ForOf { name: n, iterable, body, .. } => {
            if n.as_ref() == name {
                return false; // rebinding
            }
            expr_name_safe(iterable, name, keys) && stmt_name_safe(body, name, keys)
        }
        Statement::Return { value, .. } => opt_expr_safe(value, name, keys),
        Statement::Throw { value, .. } => expr_name_safe(value, name, keys),
        Statement::Switch { expr, cases, default_body, .. } => {
            expr_name_safe(expr, name, keys)
                && cases.iter().all(|(g, body)| {
                    g.as_ref().map(|e| expr_name_safe(e, name, keys)).unwrap_or(true)
                        && body.iter().all(|s| stmt_name_safe(s, name, keys))
                })
                && default_body
                    .as_ref()
                    .map(|b| b.iter().all(|s| stmt_name_safe(s, name, keys)))
                    .unwrap_or(true)
        }
        Statement::Try { body, catch_body, finally_body, .. } => {
            stmt_name_safe(body, name, keys)
                && catch_body.as_ref().map(|b| stmt_name_safe(b, name, keys)).unwrap_or(true)
                && finally_body.as_ref().map(|b| stmt_name_safe(b, name, keys)).unwrap_or(true)
        }
        // A nested function that closes over `name` could mutate it — bail.
        Statement::FunDecl { .. } => false,
        Statement::Break { .. } | Statement::Continue { .. } | Statement::TypeAlias { .. } => true,
        // Anything not explicitly handled: be safe, bail.
        _ => false,
    }
}

fn expr_name_safe(e: &Expr, name: &str, keys: &std::collections::HashSet<&str>) -> bool {
    use Expr::*;
    match e {
        Literal { .. } => true,
        Ident { name: n, .. } => n.as_ref() != name, // bare use of `name` is unsafe
        Member { object, prop, optional, .. } => {
            if let Ident { name: n, .. } = object.as_ref() {
                if n.as_ref() == name {
                    // `name.<prop>` — safe only as a non-optional literal-key read.
                    return !optional
                        && matches!(prop, tishlang_ast::MemberProp::Name { name: k, .. }
                            if keys.contains(k.as_ref()));
                }
            }
            expr_name_safe(object, name, keys)
                && match prop {
                    tishlang_ast::MemberProp::Expr(p) => expr_name_safe(p, name, keys),
                    tishlang_ast::MemberProp::Name { .. } => true,
                }
        }
        Binary { left, right, .. } => {
            expr_name_safe(left, name, keys) && expr_name_safe(right, name, keys)
        }
        Unary { operand, .. } | TypeOf { operand, .. } | Await { operand, .. } => {
            expr_name_safe(operand, name, keys)
        }
        Call { callee, args, .. } | New { callee, args, .. } => {
            expr_name_safe(callee, name, keys) && args.iter().all(|a| call_arg_safe(a, name, keys))
        }
        Index { object, index, .. } => {
            expr_name_safe(object, name, keys) && expr_name_safe(index, name, keys)
        }
        Conditional { cond, then_branch, else_branch, .. } => {
            expr_name_safe(cond, name, keys)
                && expr_name_safe(then_branch, name, keys)
                && expr_name_safe(else_branch, name, keys)
        }
        NullishCoalesce { left, right, .. } => {
            expr_name_safe(left, name, keys) && expr_name_safe(right, name, keys)
        }
        Array { elements, .. } => elements.iter().all(|el| match el {
            tishlang_ast::ArrayElement::Expr(e) | tishlang_ast::ArrayElement::Spread(e) => {
                expr_name_safe(e, name, keys)
            }
        }),
        Object { props, .. } => props.iter().all(|p| match p {
            tishlang_ast::ObjectProp::KeyValue(_, v) => expr_name_safe(v, name, keys),
            tishlang_ast::ObjectProp::Spread(e) => expr_name_safe(e, name, keys),
        }),
        TemplateLiteral { exprs, .. } => exprs.iter().all(|e| expr_name_safe(e, name, keys)),
        // Reassignment / mutation referencing `name` by identifier → unsafe.
        Assign { name: n, value, .. }
        | CompoundAssign { name: n, value, .. }
        | LogicalAssign { name: n, value, .. } => {
            n.as_ref() != name && expr_name_safe(value, name, keys)
        }
        PostfixInc { name: n, .. } | PostfixDec { name: n, .. } | PrefixInc { name: n, .. }
        | PrefixDec { name: n, .. } => n.as_ref() != name,
        MemberAssign { object, value, .. } => {
            // A write to `name.x` (even a literal key) is excluded in this
            // read-only version → object being `name` makes it unsafe.
            expr_name_safe(object, name, keys) && expr_name_safe(value, name, keys)
        }
        IndexAssign { object, index, value, .. } => {
            expr_name_safe(object, name, keys)
                && expr_name_safe(index, name, keys)
                && expr_name_safe(value, name, keys)
        }
        // Closures could capture+mutate `name`; JSX/native — bail conservatively.
        ArrowFunction { .. } | JsxElement { .. } | JsxFragment { .. } | NativeModuleLoad { .. } => {
            false
        }
    }
}

fn call_arg_safe(a: &CallArg, name: &str, keys: &std::collections::HashSet<&str>) -> bool {
    match a {
        CallArg::Expr(e) | CallArg::Spread(e) => expr_name_safe(e, name, keys),
    }
}

fn infer_statements(stmts: &[Statement], ctx: &mut InferCtx) -> Vec<Statement> {
    stmts.iter().map(|s| infer_statement(s, ctx)).collect()
}

fn infer_statement(stmt: &Statement, ctx: &mut InferCtx) -> Statement {
    match stmt {
        Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann,
            init,
            span,
        } => {
            // Already annotated — propagate into ctx but don't change the node.
            if let Some(ann) = type_ann {
                ctx.define(name.as_ref(), ann.clone());
                return stmt.clone();
            }
            // Try to infer from init expression.
            let inferred = init.as_ref().and_then(|e| infer_expr_type(e, ctx));
            if let Some(ref ann) = inferred {
                ctx.define(name.as_ref(), ann.clone());
            }
            Statement::VarDecl {
                name: name.clone(),
                name_span: *name_span,
                mutable: *mutable,
                type_ann: inferred,
                init: init.clone(),
                span: *span,
            }
        }
        Statement::Block { statements, span } => {
            ctx.push_scope();
            let stmts = infer_statements(statements, ctx);
            ctx.pop_scope();
            Statement::Block {
                statements: stmts,
                span: *span,
            }
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            span,
        } => {
            // Scope for loop variable
            ctx.push_scope();
            let new_init = init.as_ref().map(|i| Box::new(infer_statement(i, ctx)));
            let new_body = Box::new(infer_statement(body, ctx));
            ctx.pop_scope();
            Statement::For {
                init: new_init,
                cond: cond.clone(),
                update: update.clone(),
                body: new_body,
                span: *span,
            }
        }
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            span,
        } => {
            ctx.push_scope();
            let new_body = Box::new(infer_statement(body, ctx));
            ctx.pop_scope();
            Statement::ForOf {
                name: name.clone(),
                name_span: *name_span,
                iterable: iterable.clone(),
                body: new_body,
                span: *span,
            }
        }
        Statement::While { cond, body, span } => {
            ctx.push_scope();
            let new_body = Box::new(infer_statement(body, ctx));
            ctx.pop_scope();
            Statement::While {
                cond: cond.clone(),
                body: new_body,
                span: *span,
            }
        }
        Statement::DoWhile { body, cond, span } => {
            ctx.push_scope();
            let new_body = Box::new(infer_statement(body, ctx));
            ctx.pop_scope();
            Statement::DoWhile {
                body: new_body,
                cond: cond.clone(),
                span: *span,
            }
        }
        Statement::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => {
            let new_then = Box::new(infer_statement(then_branch, ctx));
            let new_else = else_branch
                .as_ref()
                .map(|e| Box::new(infer_statement(e, ctx)));
            Statement::If {
                cond: cond.clone(),
                then_branch: new_then,
                else_branch: new_else,
                span: *span,
            }
        }
        Statement::FunDecl {
            async_,
            name,
            name_span,
            params,
            rest_param,
            return_type,
            body,
            span,
        } => {
            ctx.push_scope();
            for p in params {
                if let FunParam::Simple(tp) = p {
                    if let Some(ann) = &tp.type_ann {
                        ctx.define(tp.name.as_ref(), ann.clone());
                    }
                }
            }
            if let Some(rp) = rest_param {
                if let Some(ann) = &rp.type_ann {
                    ctx.define(rp.name.as_ref(), ann.clone());
                }
            }
            let new_body = Box::new(infer_statement(body, ctx));
            ctx.pop_scope();
            Statement::FunDecl {
                async_: *async_,
                name: name.clone(),
                name_span: *name_span,
                params: params.clone(),
                rest_param: rest_param.clone(),
                return_type: return_type.clone(),
                body: new_body,
                span: *span,
            }
        }
        // For statements with no interesting sub-structure, clone as-is.
        _ => stmt.clone(),
    }
}

// Suppress unused import warning — CallArg is used indirectly via tishlang_ast.
#[allow(dead_code)]
fn _uses_call_arg(_: &CallArg) {}
#[allow(dead_code)]
fn _uses_arrow_body(_: &ArrowBody) {}
