//! AST-based lints for Tish. Rule IDs are stable for CI and editors.

use std::collections::HashSet;

use tishlang_ast::{Expr, ObjectProp, Program, Statement};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct LintDiagnostic {
    pub code: &'static str,
    pub message: String,
    /// 1-based line
    pub line: u32,
    /// 1-based column (UTF-16 offset preferred for LSP; we use byte col as approx)
    pub col: u32,
    pub severity: Severity,
}

/// Run all default rules on parsed program.
pub fn lint_program(program: &Program) -> Vec<LintDiagnostic> {
    let mut out = Vec::new();
    for s in &program.statements {
        lint_stmt(s, &mut out);
    }
    out
}

/// Lint source: parse then lint. Parse errors are not reported here.
pub fn lint_source(source: &str) -> Result<Vec<LintDiagnostic>, String> {
    let program = tishlang_parser::parse(source)?;
    Ok(lint_program(&program))
}

fn lint_stmt(s: &Statement, out: &mut Vec<LintDiagnostic>) {
    match s {
        Statement::Try {
            body,
            catch_param,
            catch_body,
            finally_body,
            ..
        } => {
            lint_stmt(body, out);
            if let (Some(_), Some(cb)) = (catch_param, catch_body) {
                if is_empty_block_or_stmt(cb) {
                    let span = stmt_span(cb);
                    out.push(LintDiagnostic {
                        code: "tish-empty-catch",
                        message: "Empty catch block; handle or rethrow the error.".into(),
                        line: span.0,
                        col: span.1,
                        severity: Severity::Warning,
                    });
                }
                lint_stmt(cb, out);
            }
            if let Some(fb) = finally_body {
                lint_stmt(fb, out);
            }
        }
        Statement::Block { statements, .. } => {
            for st in statements {
                lint_stmt(st, out);
            }
        }
        Statement::If {
            then_branch,
            else_branch,
            ..
        } => {
            lint_stmt(then_branch, out);
            if let Some(e) = else_branch {
                lint_stmt(e, out);
            }
        }
        Statement::While { body, .. } | Statement::ForOf { body, .. } => lint_stmt(body, out),
        Statement::For { init, body, .. } => {
            if let Some(i) = init {
                lint_stmt(i, out);
            }
            lint_stmt(body, out);
        }
        Statement::FunDecl { body, .. } => lint_stmt(body, out),
        Statement::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_, stmts) in cases {
                for st in stmts {
                    lint_stmt(st, out);
                }
            }
            if let Some(def) = default_body {
                for st in def {
                    lint_stmt(st, out);
                }
            }
        }
        Statement::DoWhile { body, .. } => lint_stmt(body, out),
        Statement::Export { declaration, .. } => {
            if let tishlang_ast::ExportDeclaration::Named(inner) = declaration.as_ref() {
                lint_stmt(inner, out);
            }
        }
        Statement::ExprStmt { expr, .. } => lint_expr(expr, out),
        Statement::VarDecl { init, .. } => {
            if let Some(e) = init {
                lint_expr(e, out);
            }
        }
        Statement::VarDeclDestructure { init, .. } => lint_expr(init, out),
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                lint_expr(e, out);
            }
        }
        Statement::Throw { value, .. } => lint_expr(value, out),
        _ => {}
    }
}

fn is_empty_block_or_stmt(s: &Statement) -> bool {
    match s {
        Statement::Block { statements, .. } => statements.is_empty(),
        Statement::ExprStmt { .. } => false,
        _ => false,
    }
}

fn stmt_span(s: &Statement) -> (u32, u32) {
    let sp = match s {
        Statement::Block { span, .. } => *span,
        Statement::Try { span, .. } => *span,
        _ => tishlang_ast::Span {
            start: (1, 1),
            end: (1, 1),
        },
    };
    (sp.start.0 as u32, sp.start.1 as u32)
}

fn lint_expr(e: &Expr, out: &mut Vec<LintDiagnostic>) {
    match e {
        Expr::Object { props, span, .. } => {
            let mut seen: HashSet<&str> = HashSet::new();
            for p in props {
                if let ObjectProp::KeyValue(k, v) = p {
                    if !seen.insert(k.as_ref()) {
                        out.push(LintDiagnostic {
                            code: "tish-duplicate-key",
                            message: format!("Duplicate object key `{}`", k),
                            line: span.start.0 as u32,
                            col: span.start.1 as u32,
                            severity: Severity::Warning,
                        });
                    }
                    lint_expr(v, out);
                } else if let ObjectProp::Spread(ex) = p {
                    lint_expr(ex, out);
                }
            }
        }
        Expr::Binary { left, right, .. } => {
            lint_expr(left, out);
            lint_expr(right, out);
        }
        Expr::Unary { operand, .. } => lint_expr(operand, out),
        Expr::Call { callee, args, .. } => {
            lint_expr(callee, out);
            for a in args {
                match a {
                    tishlang_ast::CallArg::Expr(x) => lint_expr(x, out),
                    tishlang_ast::CallArg::Spread(x) => lint_expr(x, out),
                }
            }
        }
        Expr::New { callee, args, .. } => {
            lint_expr(callee, out);
            for a in args {
                match a {
                    tishlang_ast::CallArg::Expr(x) => lint_expr(x, out),
                    tishlang_ast::CallArg::Spread(x) => lint_expr(x, out),
                }
            }
        }
        Expr::Member { object, .. } => {
            lint_expr(object, out);
        }
        Expr::Index { object, index, .. } => {
            lint_expr(object, out);
            lint_expr(index, out);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            lint_expr(cond, out);
            lint_expr(then_branch, out);
            lint_expr(else_branch, out);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            lint_expr(left, out);
            lint_expr(right, out);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(x) => lint_expr(x, out),
                    tishlang_ast::ArrayElement::Spread(x) => lint_expr(x, out),
                }
            }
        }
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. } => lint_expr(value, out),
        Expr::MemberAssign { object, value, .. } => {
            lint_expr(object, out);
            lint_expr(value, out);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            lint_expr(object, out);
            lint_expr(index, out);
            lint_expr(value, out);
        }
        Expr::ArrowFunction { body, .. } => match body {
            tishlang_ast::ArrowBody::Expr(x) => lint_expr(x, out),
            tishlang_ast::ArrowBody::Block(b) => lint_stmt(b, out),
        },
        Expr::TemplateLiteral { exprs, .. } => {
            for x in exprs {
                lint_expr(x, out);
            }
        }
        Expr::Await { operand, .. } => lint_expr(operand, out),
        Expr::TypeOf { operand, .. } => lint_expr(operand, out),
        Expr::JsxElement {
            props, children, ..
        } => {
            for pr in props {
                match pr {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(x) = value {
                            lint_expr(x, out);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(x) => lint_expr(x, out),
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(x) = ch {
                    lint_expr(x, out);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(x) = ch {
                    lint_expr(x, out);
                }
            }
        }
        _ => {}
    }
}

/// Human-readable catalog for documentation.
pub const RULES: &[(&str, &str)] = &[
    (
        "tish-empty-catch",
        "Warns on catch blocks with no statements (likely mistake).",
    ),
    (
        "tish-duplicate-key",
        "Warns when an object literal repeats the same key.",
    ),
];
