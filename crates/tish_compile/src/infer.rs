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
    Program {
        statements: infer_statements(&program.statements, &mut ctx),
    }
}

fn infer_statements(stmts: &[Statement], ctx: &mut InferCtx) -> Vec<Statement> {
    stmts.iter().map(|s| infer_statement(s, ctx)).collect()
}

fn infer_statement(stmt: &Statement, ctx: &mut InferCtx) -> Statement {
    match stmt {
        Statement::VarDecl {
            name,
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
            iterable,
            body,
            span,
        } => {
            ctx.push_scope();
            let new_body = Box::new(infer_statement(body, ctx));
            ctx.pop_scope();
            Statement::ForOf {
                name: name.clone(),
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
