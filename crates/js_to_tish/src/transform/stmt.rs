//! Convert OXC statements to Tish statements.

use std::sync::Arc;

use oxc::ast::ast::Statement as OxcStmt;
use oxc::semantic::Semantic;
use tishlang_ast::{Statement, Span};

use super::expr;
use crate::error::{ConvertError, ConvertErrorKind};
use crate::span_util;

type Ctx<'a> = (&'a Semantic<'a>, &'a str);

/// Convert OXC program body (statements) to Tish statements.
pub fn convert_statements(
    body: &[OxcStmt<'_>],
    semantic: &Semantic<'_>,
    source: &str,
) -> Result<Vec<Statement>, ConvertError> {
    let ctx = (semantic, source);
    body.iter()
        .map(|s| convert_statement(s, &ctx))
        .collect()
}

fn convert_statement(stmt: &OxcStmt<'_>, ctx: &Ctx<'_>) -> Result<Statement, ConvertError> {
    let span = span_util::oxc_span_to_tish(ctx.1, stmt);
    match stmt {
        OxcStmt::BlockStatement(b) => {
            let statements = convert_statements(&b.body, ctx.0, ctx.1)?;
            Ok(Statement::Block {
                statements,
                span,
            })
        }
        OxcStmt::VariableDeclaration(v) => convert_var_decl(v, ctx, span),
        OxcStmt::ExpressionStatement(e) => {
            let expr = expr::convert_expr(&e.expression, ctx)?;
            Ok(Statement::ExprStmt { expr, span })
        }
        OxcStmt::IfStatement(i) => {
            let cond = expr::convert_expr(&i.test, ctx)?;
            let then_branch = Box::new(convert_statement(&i.consequent, ctx)?);
            let else_branch = i
                .alternate
                .as_ref()
                .map(|a| convert_statement(a, ctx))
                .transpose()?
                .map(Box::new);
            Ok(Statement::If {
                cond,
                then_branch,
                else_branch,
                span,
            })
        }
        OxcStmt::ReturnStatement(r) => {
            let value = r
                .argument
                .as_ref()
                .map(|a| expr::convert_expr(a, ctx))
                .transpose()?;
            Ok(Statement::Return { value, span })
        }
        OxcStmt::BreakStatement(_) => Ok(Statement::Break { span }),
        OxcStmt::ContinueStatement(_) => Ok(Statement::Continue { span }),
        OxcStmt::WhileStatement(w) => {
            let cond = expr::convert_expr(&w.test, ctx)?;
            let body = Box::new(convert_statement(&w.body, ctx)?);
            Ok(Statement::While { cond, body, span })
        }
        OxcStmt::DoWhileStatement(d) => {
            let cond = expr::convert_expr(&d.test, ctx)?;
            let body = Box::new(convert_statement(&d.body, ctx)?);
            Ok(Statement::DoWhile { body, cond, span })
        }
        OxcStmt::ForStatement(f) => convert_for_statement(f, ctx, span),
        OxcStmt::ForOfStatement(f) => convert_for_of_statement(f, ctx, span),
        OxcStmt::SwitchStatement(s) => convert_switch_statement(s, ctx, span),
        OxcStmt::ThrowStatement(t) => {
            let value = expr::convert_expr(&t.argument, ctx)?;
            Ok(Statement::Throw { value, span })
        }
        OxcStmt::TryStatement(t) => convert_try_statement(t, ctx, span),
        OxcStmt::FunctionDeclaration(f) => convert_function_decl(f, ctx, span),
        OxcStmt::EmptyStatement(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "empty statement".into(),
            hint: None,
        })),
        OxcStmt::ForInStatement(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "for-in".into(),
            hint: Some("Tish omits for-in".into()),
        })),
        OxcStmt::ClassDeclaration(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "class".into(),
            hint: Some("Tish does not support classes".into()),
        })),
        OxcStmt::WithStatement(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "with".into(),
            hint: None,
        })),
        OxcStmt::DebuggerStatement(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "debugger".into(),
            hint: None,
        })),
        OxcStmt::LabeledStatement(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "labeled statement".into(),
            hint: None,
        })),
        OxcStmt::ImportDeclaration(i) => convert_import(i, ctx, span),
        OxcStmt::ExportDefaultDeclaration(e) => convert_export_default(e, ctx, span),
        OxcStmt::ExportNamedDeclaration(e) => convert_export_named(e, ctx, span),
        OxcStmt::ExportAllDeclaration(_) => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "export *".into(),
            hint: None,
        })),
        _ => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: format!("{:?}", std::mem::discriminant(stmt)),
            hint: None,
        })),
    }
}

fn convert_declaration(
    decl: &oxc::ast::ast::Declaration<'_>,
    ctx: &Ctx<'_>,
) -> Result<Statement, ConvertError> {
    let span = span_util::oxc_span_to_tish(ctx.1, decl);
    match decl {
        oxc::ast::ast::Declaration::VariableDeclaration(v) => convert_var_decl(v, ctx, span),
        oxc::ast::ast::Declaration::FunctionDeclaration(f) => convert_function_decl(f, ctx, span),
        _ => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: format!("declaration: {:?}", std::mem::discriminant(decl)),
            hint: None,
        })),
    }
}

fn convert_var_decl(
    v: &oxc::ast::ast::VariableDeclaration<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let mutable = matches!(v.kind, oxc::ast::ast::VariableDeclarationKind::Let);
    if v.declarations.len() == 1 {
        let d = &v.declarations[0];
        let id = &d.id;
        let init = d.init.as_ref().map(|i| expr::convert_expr(i, ctx)).transpose()?;
        match id {
            oxc::ast::ast::BindingPattern::BindingIdentifier(b) => {
                let name: Arc<str> = Arc::from(b.name.as_str());
                Ok(Statement::VarDecl {
                    name,
                    mutable,
                    type_ann: None,
                    init,
                    span,
                })
            }
            _ => {
                let init = d
                    .init
                    .as_ref()
                    .map(|i| expr::convert_expr(i, ctx))
                    .transpose()?
                    .ok_or_else(|| {
                        ConvertError::new(ConvertErrorKind::Incompatible {
                            what: "destructuring declaration".into(),
                            reason: "initializer required".into(),
                        })
                    })?;
                let pattern = expr::convert_destruct_pattern(id)?;
                Ok(Statement::VarDeclDestructure {
                    pattern,
                    mutable,
                    init,
                    span,
                })
            }
        }
    } else {
        Err(ConvertError::new(ConvertErrorKind::Incompatible {
            what: "multi-declarator variable declaration".into(),
            reason: "split into separate declarations".into(),
        }))
    }
}

fn convert_for_statement(
    f: &oxc::ast::ast::ForStatement<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let init = f
        .init
        .as_ref()
        .map(|i| match i {
            oxc::ast::ast::ForStatementInit::VariableDeclaration(v) => {
                convert_var_decl(v, ctx, span_util::stub_span()).map(Box::new)
            }
            _ => {
                if let Some(e) = i.as_expression() {
                    expr::convert_expr(e, ctx).map(|expr| {
                        Box::new(Statement::ExprStmt {
                            expr,
                            span: span_util::stub_span(),
                        })
                    })
                } else {
                    Err(ConvertError::new(ConvertErrorKind::Unsupported {
                        what: "for init".into(),
                        hint: None,
                    }))
                }
            }
        })
        .transpose()?;
    let cond = f
        .test
        .as_ref()
        .map(|e| expr::convert_expr(e, ctx))
        .transpose()?;
    let update = f
        .update
        .as_ref()
        .map(|e| expr::convert_expr(e, ctx))
        .transpose()?;
    let body = Box::new(convert_statement(&f.body, ctx)?);
    Ok(Statement::For {
        init,
        cond,
        update,
        body,
        span,
    })
}

fn convert_for_of_statement(
    f: &oxc::ast::ast::ForOfStatement<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let name = match &f.left {
        oxc::ast::ast::ForStatementLeft::VariableDeclaration(v) => {
            if v.declarations.len() == 1 {
                let d = &v.declarations[0];
                match &d.id {
                    oxc::ast::ast::BindingPattern::BindingIdentifier(b) => b.name.as_str(),
                    _ => {
                        return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                            what: "for-of with destructuring".into(),
                            reason: "use simple identifier".into(),
                        }))
                    }
                }
            } else {
                return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                    what: "for-of with multiple bindings".into(),
                    reason: "not supported".into(),
                }));
            }
        }
        _ => {
            return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                what: "for-of (use variable declaration in left)".into(),
                reason: "e.g. for (const x of arr)".into(),
            }))
        }
    };
    let iterable = expr::convert_expr(&f.right, ctx)?;
    let body = Box::new(convert_statement(&f.body, ctx)?);
    Ok(Statement::ForOf {
        name: Arc::from(name),
        iterable,
        body,
        span,
    })
}

fn convert_switch_statement(
    s: &oxc::ast::ast::SwitchStatement<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let expr = expr::convert_expr(&s.discriminant, ctx)?;
    let mut cases = Vec::new();
    let mut default_body: Option<Vec<Statement>> = None;
    for c in &s.cases {
        let stmts = convert_statements(&c.consequent, ctx.0, ctx.1)?;
        match &c.test {
            Some(t) => cases.push((Some(expr::convert_expr(t, ctx)?), stmts)),
            None => default_body = Some(stmts),
        }
    }
    Ok(Statement::Switch {
        expr,
        cases,
        default_body,
        span,
    })
}

fn convert_try_statement(
    t: &oxc::ast::ast::TryStatement<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    // TryStatement.block is BlockStatement; convert its body to Statement::Block
    let body_stmts = convert_statements(&t.block.body, ctx.0, ctx.1)?;
    let body = Box::new(Statement::Block {
        statements: body_stmts,
        span: span_util::oxc_span_to_tish(ctx.1, &*t.block),
    });
    let (catch_param, catch_body) = match &t.handler {
        Some(h) => {
            let param = h.param.as_ref().and_then(|cp: &oxc::ast::ast::CatchParameter<'_>| {
                if let oxc::ast::ast::BindingPattern::BindingIdentifier(b) = &cp.pattern {
                    Some(Arc::from(b.name.as_str()))
                } else {
                    None
                }
            });
            let catch_stmts = convert_statements(&h.body.body, ctx.0, ctx.1)?;
            let cb = Box::new(Statement::Block {
                statements: catch_stmts,
                span: span_util::oxc_span_to_tish(ctx.1, &*h.body),
            });
            (param, Some(cb))
        }
        None => (None, None),
    };
    let finally_body = t
        .finalizer
        .as_ref()
        .map(|f| {
            let stmts = convert_statements(&f.body, ctx.0, ctx.1)?;
            Ok(Box::new(Statement::Block {
                statements: stmts,
                span: span_util::oxc_span_to_tish(ctx.1, &**f),
            }))
        })
        .transpose()?;
    Ok(Statement::Try {
        body,
        catch_param,
        catch_body,
        finally_body,
        span,
    })
}

fn convert_function_decl(
    f: &oxc::ast::ast::Function<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let async_ = f.r#async;
    let name: Arc<str> = f
        .id
        .as_ref()
        .map(|id| Arc::from(id.name.as_str()))
        .unwrap_or_else(|| Arc::from(""));
    let (params, rest_param) = expr::convert_params(&f.params, ctx)?;
    let body = match &f.body {
        Some(fb) => {
            let stmts = convert_statements(&fb.statements, ctx.0, ctx.1)?;
            Box::new(Statement::Block {
                statements: stmts,
                span: span_util::oxc_span_to_tish(ctx.1, fb.as_ref()),
            })
        }
        None => {
            return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                what: "function body".into(),
                reason: "expected block".into(),
            }))
        }
    };
    Ok(Statement::FunDecl {
        async_,
        name,
        params,
        rest_param,
        return_type: None,
        body,
        span,
    })
}

fn convert_import(
    i: &oxc::ast::ast::ImportDeclaration<'_>,
    _ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let from: Arc<str> = Arc::from(i.source.value.as_str());
    let mut specifiers = Vec::new();
    if let Some(specs) = &i.specifiers {
        for s in specs.iter() {
            match s {
                oxc::ast::ast::ImportDeclarationSpecifier::ImportSpecifier(is) => {
                    let imported_name = is.imported.name().as_str();
                    let local_name = is.local.name.as_str();
                    let alias = if imported_name == local_name {
                        None
                    } else {
                        Some(Arc::from(local_name))
                    };
                    specifiers.push(tishlang_ast::ImportSpecifier::Named {
                        name: Arc::from(imported_name),
                        alias,
                    });
                }
                oxc::ast::ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(ds) => {
                    specifiers.push(tishlang_ast::ImportSpecifier::Default(Arc::from(
                        ds.local.name.as_str(),
                    )));
                }
                oxc::ast::ast::ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns) => {
                    specifiers.push(tishlang_ast::ImportSpecifier::Namespace(Arc::from(
                        ns.local.name.as_str(),
                    )));
                }
            }
        }
    }
    Ok(Statement::Import {
        specifiers,
        from,
        span,
    })
}

fn convert_export_default(
    e: &oxc::ast::ast::ExportDefaultDeclaration<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    let declaration = if let Some(expr) = e.declaration.as_expression() {
        let expr = expr::convert_expr(expr, ctx)?;
        tishlang_ast::ExportDeclaration::Default(expr)
    } else if let oxc::ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(f) = &e.declaration {
        let stmt = convert_function_decl(f.as_ref(), ctx, span_util::stub_span())?;
        tishlang_ast::ExportDeclaration::Named(Box::new(stmt))
    } else {
        return Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "export default (this form)".into(),
            hint: None,
        }));
    };
    Ok(Statement::Export {
        declaration: Box::new(declaration),
        span,
    })
}

fn convert_export_named(
    e: &oxc::ast::ast::ExportNamedDeclaration<'_>,
    ctx: &Ctx<'_>,
    span: Span,
) -> Result<Statement, ConvertError> {
    if let Some(decl) = &e.declaration {
        let stmt = convert_declaration(decl, ctx)?;
        Ok(Statement::Export {
            declaration: Box::new(tishlang_ast::ExportDeclaration::Named(Box::new(stmt))),
            span,
        })
    } else {
        Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "export { ... } (re-exports)".into(),
            hint: None,
        }))
    }
}
