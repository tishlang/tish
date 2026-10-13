//! Whether code `await`s — shared by the backends that need to know (the native backend wires its
//! promise runtime in only for programs that await; the JS backend marks a function `async` when its
//! own body awaits, because Tish allows `await` in any function and JS only in `async` ones).

use crate::*;

pub fn expr_awaits(e: &Expr, into_functions: bool) -> bool {
    let expr_has_await = |e: &Expr| expr_awaits(e, into_functions);
    let stmt_has_await = |s: &Statement| stmt_awaits(s, into_functions);
    match e {
        Expr::Await { .. } => true,
        Expr::Binary { left, right, .. } => expr_has_await(left) || expr_has_await(right),
        Expr::Unary { operand, .. } | Expr::TypeOf { operand, .. } => expr_has_await(operand),
        Expr::Call { callee, args, .. } => {
            expr_has_await(callee)
                || args.iter().any(|a| match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => expr_has_await(e),
                })
        }
        Expr::New { callee, args, .. } => {
            expr_has_await(callee)
                || args.iter().any(|a| match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => expr_has_await(e),
                })
        }
        Expr::Member { object, prop, .. } => {
            expr_has_await(object)
                || if let MemberProp::Expr(e) = prop {
                    expr_has_await(e)
                } else {
                    false
                }
        }
        Expr::Index { object, index, .. } => expr_has_await(object) || expr_has_await(index),
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => expr_has_await(cond) || expr_has_await(then_branch) || expr_has_await(else_branch),
        Expr::NullishCoalesce { left, right, .. } => {
            expr_has_await(left) || expr_has_await(right)
        }
        Expr::Array { elements, .. } => elements.iter().any(|el| match el {
            ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_has_await(e),
        }),
        Expr::Object { props, .. } => props.iter().any(|p| match p {
            ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => expr_has_await(e),
        }),
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. }
        | Expr::MemberAssign { value, .. }
        | Expr::IndexAssign { value, .. } => expr_has_await(value),
        Expr::ArrowFunction { body, .. } => {
            into_functions
                && match body {
                    ArrowBody::Expr(e) => expr_has_await(e),
                    ArrowBody::Block(s) => stmt_has_await(s),
                }
        }
        Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(expr_has_await),
        Expr::JsxElement {
            props, children, ..
        } => {
            props.iter().any(|p| match p {
                crate::JsxProp::Attr {
                    value: crate::JsxAttrValue::Expr(e),
                    ..
                }
                | crate::JsxProp::Spread(e) => expr_has_await(e),
                _ => false,
            }) || children
                .iter()
                .any(|c| matches!(c, crate::JsxChild::Expr(e) if expr_has_await(e)))
        }
        Expr::JsxFragment { children, .. } => children
            .iter()
            .any(|c| matches!(c, crate::JsxChild::Expr(e) if expr_has_await(e))),
        _ => false,
    }
}
pub fn stmt_awaits(s: &Statement, into_functions: bool) -> bool {
    let expr_has_await = |e: &Expr| expr_awaits(e, into_functions);
    let stmt_has_await = |s: &Statement| stmt_awaits(s, into_functions);
    match s {
        Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
            statements.iter().any(stmt_has_await)
        }
        Statement::VarDecl { init, .. } => init.as_ref().is_some_and(expr_has_await),
        Statement::VarDeclDestructure { init, .. } => expr_has_await(init),
        Statement::ExprStmt { expr, .. } => expr_has_await(expr),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_has_await(cond)
                || stmt_has_await(then_branch)
                || else_branch
                    .as_ref()
                    .is_some_and(|s| stmt_has_await(s.as_ref()))
        }
        Statement::While { cond, body, .. } => expr_has_await(cond) || stmt_has_await(body),
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(|s| stmt_has_await(s.as_ref()))
                || cond.as_ref().is_some_and(expr_has_await)
                || update.as_ref().is_some_and(expr_has_await)
                || stmt_has_await(body)
        }
        Statement::ForOf { iterable, body, .. }
        | Statement::ForIn {
            object: iterable,
            body,
            ..
        } => {
            expr_has_await(iterable) || stmt_has_await(body)
        }
        Statement::Return { value, .. } => value.as_ref().is_some_and(expr_has_await),
        Statement::FunDecl { body, .. } => into_functions && stmt_has_await(body),
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            expr_has_await(expr)
                || cases.iter().any(|(c, stmts)| {
                    c.as_ref().is_some_and(expr_has_await) || stmts.iter().any(stmt_has_await)
                })
                || default_body
                    .as_ref()
                    .is_some_and(|stmts| stmts.iter().any(stmt_has_await))
        }
        Statement::DoWhile { body, cond, .. } => stmt_has_await(body) || expr_has_await(cond),
        Statement::Throw { value, .. } => expr_has_await(value),
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            stmt_has_await(body)
                || catch_body
                    .as_ref()
                    .is_some_and(|s| stmt_has_await(s.as_ref()))
                || finally_body
                    .as_ref()
                    .is_some_and(|s| stmt_has_await(s.as_ref()))
        }
        Statement::Import { .. } | Statement::Export { .. } => false,
        _ => false,
    }
}
