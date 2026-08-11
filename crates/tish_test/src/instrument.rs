//! AST instrumentation for line coverage — rewrite programs in `tish_test` only.

use std::path::Path;
use std::sync::Arc;

use tishlang_ast::{
    ArrowBody, CallArg, ExportDeclaration, Expr, Literal, Program, Span, Statement,
};

use crate::coverage::{self, hit_global_name};

/// Instrument every resolved module's AST so execution of any backend records line hits.
pub fn instrument_program(program: &mut Program, path: &Path) {
    if !coverage::is_enabled() {
        return;
    }
    let file_id = coverage::intern_file(path);
    program.statements = instrument_stmts(std::mem::take(&mut program.statements), file_id);
}

fn instrument_stmts(stmts: Vec<Statement>, file_id: u32) -> Vec<Statement> {
    let mut out = Vec::with_capacity(stmts.len() * 2);
    for stmt in stmts {
        match stmt {
            Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. }
            | Statement::Import { .. } => {
                out.push(stmt);
            }
            Statement::Export {
                declaration,
                span,
            } => {
                let declaration = Box::new(match *declaration {
                    ExportDeclaration::Named(inner) => {
                        ExportDeclaration::Named(Box::new(instrument_stmt(*inner, file_id)))
                    }
                    ExportDeclaration::Default(expr) => {
                        ExportDeclaration::Default(instrument_expr(expr, file_id))
                    }
                    other => other,
                });
                out.push(hit_stmt(file_id, span.start.0 as u32));
                out.push(Statement::Export {
                    declaration,
                    span,
                });
            }
            Statement::Multi { statements, span } => {
                out.push(Statement::Multi {
                    statements: instrument_stmts(statements, file_id),
                    span,
                });
            }
            Statement::Block { statements, span } => {
                out.push(Statement::Block {
                    statements: instrument_stmts(statements, file_id),
                    span,
                });
            }
            other => {
                let line = other.span().start.0 as u32;
                out.push(hit_stmt(file_id, line));
                out.push(instrument_stmt(other, file_id));
            }
        }
    }
    out
}

fn instrument_stmt(stmt: Statement, file_id: u32) -> Statement {
    match stmt {
        Statement::Block { statements, span } => Statement::Block {
            statements: instrument_stmts(statements, file_id),
            span,
        },
        Statement::Multi { statements, span } => Statement::Multi {
            statements: instrument_stmts(statements, file_id),
            span,
        },
        Statement::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => Statement::If {
            cond: instrument_expr(cond, file_id),
            then_branch: Box::new(instrument_stmt(*then_branch, file_id)),
            else_branch: else_branch.map(|b| Box::new(instrument_stmt(*b, file_id))),
            span,
        },
        Statement::While { cond, body, span } => Statement::While {
            cond: instrument_expr(cond, file_id),
            body: Box::new(instrument_stmt(*body, file_id)),
            span,
        },
        Statement::DoWhile { body, cond, span } => Statement::DoWhile {
            body: Box::new(instrument_stmt(*body, file_id)),
            cond: instrument_expr(cond, file_id),
            span,
        },
        Statement::For {
            init,
            cond,
            update,
            body,
            span,
        } => Statement::For {
            init: init.map(|i| Box::new(instrument_stmt(*i, file_id))),
            cond: cond.map(|c| instrument_expr(c, file_id)),
            update: update.map(|u| instrument_expr(u, file_id)),
            body: Box::new(instrument_stmt(*body, file_id)),
            span,
        },
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            span,
        } => Statement::ForOf {
            name,
            name_span,
            iterable: instrument_expr(iterable, file_id),
            body: Box::new(instrument_stmt(*body, file_id)),
            span,
        },
        Statement::ForIn {
            name,
            name_span,
            object,
            body,
            span,
        } => Statement::ForIn {
            name,
            name_span,
            object: instrument_expr(object, file_id),
            body: Box::new(instrument_stmt(*body, file_id)),
            span,
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
            async_,
            name,
            name_span,
            params,
            rest_param,
            return_type,
            body: Box::new(instrument_stmt(*body, file_id)),
            span,
        },
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            span,
        } => Statement::Try {
            body: Box::new(instrument_stmt(*body, file_id)),
            catch_param,
            catch_param_span,
            catch_body: catch_body.map(|b| Box::new(instrument_stmt(*b, file_id))),
            finally_body: finally_body.map(|b| Box::new(instrument_stmt(*b, file_id))),
            span,
        },
        Statement::Switch {
            expr,
            cases,
            default_body,
            span,
        } => Statement::Switch {
            expr: instrument_expr(expr, file_id),
            cases: cases
                .into_iter()
                .map(|(c, body)| (c.map(|e| instrument_expr(e, file_id)), instrument_stmts(body, file_id)))
                .collect(),
            default_body: default_body.map(|b| instrument_stmts(b, file_id)),
            span,
        },
        Statement::ExprStmt { expr, span } => Statement::ExprStmt {
            expr: instrument_expr(expr, file_id),
            span,
        },
        Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann,
            init,
            span,
        } => Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann,
            init: init.map(|e| instrument_expr(e, file_id)),
            span,
        },
        Statement::VarDeclDestructure {
            pattern,
            mutable,
            init,
            span,
        } => Statement::VarDeclDestructure {
            pattern,
            mutable,
            init: instrument_expr(init, file_id),
            span,
        },
        Statement::Return { value, span } => Statement::Return {
            value: value.map(|e| instrument_expr(e, file_id)),
            span,
        },
        Statement::Throw { value, span } => Statement::Throw {
            value: instrument_expr(value, file_id),
            span,
        },
        other => other,
    }
}

fn instrument_expr(expr: Expr, file_id: u32) -> Expr {
    match expr {
        Expr::ArrowFunction {
            async_,
            params,
            body,
            span,
        } => Expr::ArrowFunction {
            async_,
            params,
            body: match body {
                ArrowBody::Expr(e) => ArrowBody::Expr(Box::new(instrument_expr(*e, file_id))),
                ArrowBody::Block(b) => ArrowBody::Block(Box::new(instrument_stmt(*b, file_id))),
            },
            span,
        },
        Expr::Call { callee, args, span } => Expr::Call {
            callee: Box::new(instrument_expr(*callee, file_id)),
            args: args
                .into_iter()
                .map(|a| match a {
                    CallArg::Expr(e) => CallArg::Expr(instrument_expr(e, file_id)),
                    CallArg::Spread(e) => CallArg::Spread(instrument_expr(e, file_id)),
                })
                .collect(),
            span,
        },
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => Expr::Binary {
            left: Box::new(instrument_expr(*left, file_id)),
            op,
            right: Box::new(instrument_expr(*right, file_id)),
            span,
        },
        Expr::Unary { op, operand, span } => Expr::Unary {
            op,
            operand: Box::new(instrument_expr(*operand, file_id)),
            span,
        },
        Expr::Member {
            object,
            prop,
            optional,
            span,
        } => Expr::Member {
            object: Box::new(instrument_expr(*object, file_id)),
            prop,
            optional,
            span,
        },
        Expr::Assign { name, value, span } => Expr::Assign {
            name,
            value: Box::new(instrument_expr(*value, file_id)),
            span,
        },
        Expr::Array { elements, span } => Expr::Array {
            elements: elements
                .into_iter()
                .map(|el| match el {
                    tishlang_ast::ArrayElement::Expr(e) => {
                        tishlang_ast::ArrayElement::Expr(instrument_expr(e, file_id))
                    }
                    tishlang_ast::ArrayElement::Spread(e) => {
                        tishlang_ast::ArrayElement::Spread(instrument_expr(e, file_id))
                    }
                })
                .collect(),
            span,
        },
        Expr::Object { props, span } => Expr::Object {
            props: props
                .into_iter()
                .map(|p| match p {
                    tishlang_ast::ObjectProp::KeyValue(k, v, s) => {
                        tishlang_ast::ObjectProp::KeyValue(k, instrument_expr(v, file_id), s)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        tishlang_ast::ObjectProp::Spread(instrument_expr(e, file_id))
                    }
                })
                .collect(),
            span,
        },
        other => other,
    }
}

fn hit_stmt(file_id: u32, line: u32) -> Statement {
    coverage::mark_executable(file_id, line);
    let span = Span {
        start: (line as usize, 1),
        end: (line as usize, 1),
    };
    let name: Arc<str> = Arc::from(hit_global_name());
    Statement::ExprStmt {
        expr: Expr::Call {
            callee: Box::new(Expr::Ident {
                name,
                span,
            }),
            args: vec![
                CallArg::Expr(Expr::Literal {
                    value: Literal::Number(file_id as f64),
                    span,
                }),
                CallArg::Expr(Expr::Literal {
                    value: Literal::Number(line as f64),
                    span,
                }),
            ],
            span,
        },
        span,
    }
}
