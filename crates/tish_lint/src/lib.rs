//! AST-based lints for Tish. Rule IDs are stable for CI and editors.

use std::collections::HashSet;

use tishlang_ast::{Expr, Literal, ObjectProp, Program, Statement};

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

/// Like [`lint_program`], plus name-resolution diagnostics from `tishlang_resolve`
/// (unused imports/parameters/variables and unresolved identifiers). Results are sorted
/// by source position. `source` is required for the resolver's span math.
pub fn lint_program_with_resolution(program: &Program, source: &str) -> Vec<LintDiagnostic> {
    let mut out = lint_program(program);
    for b in tishlang_resolve::collect_unused_bindings(program, source) {
        let (code, what) = match b.kind {
            tishlang_resolve::UnusedBindingKind::Import => {
                ("tish-unused-import", "imported but never used")
            }
            tishlang_resolve::UnusedBindingKind::Parameter => {
                ("tish-unused-parameter", "declared but never read")
            }
            tishlang_resolve::UnusedBindingKind::Variable => {
                ("tish-unused-variable", "declared but its value is never read")
            }
        };
        out.push(LintDiagnostic {
            code,
            message: format!("`{}` is {what}", b.name),
            line: b.span.start.0 as u32,
            col: b.span.start.1 as u32,
            severity: Severity::Warning,
        });
    }
    for u in tishlang_resolve::collect_unresolved_identifiers(program) {
        out.push(LintDiagnostic {
            code: "tish-unresolved-name",
            message: format!("no binding in scope for `{}`", u.name),
            line: u.span.start.0 as u32,
            col: u.span.start.1 as u32,
            severity: Severity::Warning,
        });
    }
    out.sort_by_key(|d| (d.line, d.col));
    out
}

/// Parse then lint with resolution diagnostics (see [`lint_program_with_resolution`]).
pub fn lint_source_with_resolution(source: &str) -> Result<Vec<LintDiagnostic>, String> {
    let program = tishlang_parser::parse(source)?;
    Ok(lint_program_with_resolution(&program, source))
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
        Expr::TemplateLiteral { quasis, exprs, span } => {
            for q in quasis {
                if let Some(label) = detect_secret(q) {
                    out.push(secret_diag(label, span));
                }
            }
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
        Expr::Literal {
            value: Literal::String(s),
            span,
        } => {
            if let Some(label) = detect_secret(s) {
                out.push(secret_diag(label, span));
            }
        }
        _ => {}
    }
}

/// Returns a provider label when `s` looks like a hardcoded credential.
fn detect_secret(s: &str) -> Option<&'static str> {
    if s.strip_prefix("sk-live-").is_some_and(|rest| !rest.is_empty()) {
        return Some("sk-live- API key");
    }
    if s.len() == 20
        && s.starts_with("AKIA")
        && s.as_bytes()[4..]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Some("AWS access key id");
    }
    if s.strip_prefix("ghp_")
        .is_some_and(|rest| rest.len() >= 20 && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Some("GitHub token");
    }
    None
}

fn secret_diag(label: &str, span: &tishlang_ast::Span) -> LintDiagnostic {
    LintDiagnostic {
        code: "tish-hardcoded-secret",
        message: format!(
            "Possible hardcoded secret ({label}); load it from the environment instead."
        ),
        line: span.start.0 as u32,
        col: span.start.1 as u32,
        severity: Severity::Error,
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
    (
        "tish-hardcoded-secret",
        "Flags string literals that look like hardcoded credentials (sk-live-, AWS, GitHub tokens).",
    ),
    (
        "tish-unused-import",
        "Warns when an import binding is never used.",
    ),
    (
        "tish-unused-parameter",
        "Warns when a function parameter is never read.",
    ),
    (
        "tish-unused-variable",
        "Warns when a variable's value is never read.",
    ),
    (
        "tish-unresolved-name",
        "Warns when an identifier has no binding in scope.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(src: &str) -> Vec<&'static str> {
        lint_source(src)
            .expect("parse")
            .into_iter()
            .map(|d| d.code)
            .collect()
    }

    fn codes_resolved(src: &str) -> Vec<&'static str> {
        lint_source_with_resolution(src)
            .expect("parse")
            .into_iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn empty_catch_flagged() {
        let src = "fn risky() {\n  try {\n    mightThrow()\n  } catch (e) {\n  }\n}\n";
        assert!(codes(src).contains(&"tish-empty-catch"));
    }

    #[test]
    fn empty_catch_clean_when_handled() {
        let src =
            "fn risky() {\n  try {\n    mightThrow()\n  } catch (e) {\n    console.error(e)\n  }\n}\n";
        assert!(!codes(src).contains(&"tish-empty-catch"));
    }

    #[test]
    fn duplicate_key_flagged() {
        assert!(codes("let o = { a: 1, a: 2 }\n").contains(&"tish-duplicate-key"));
    }

    #[test]
    fn duplicate_key_clean() {
        assert!(!codes("let o = { a: 2 }\n").contains(&"tish-duplicate-key"));
    }

    #[test]
    fn hardcoded_secret_flagged() {
        let src = "let apiKey = \"sk-live-000000000000000000000000\"\n";
        assert!(codes(src).contains(&"tish-hardcoded-secret"));
    }

    #[test]
    fn ordinary_string_not_flagged() {
        assert!(!codes("let s = \"hello world\"\n").contains(&"tish-hardcoded-secret"));
    }

    #[test]
    fn detect_secret_providers() {
        assert!(detect_secret("sk-live-abc").is_some());
        assert!(detect_secret("AKIAIOSFODNN7EXAMPLE").is_some());
        assert!(detect_secret("ghp_0123456789abcdef0123456789abcdef0123").is_some());
        assert!(detect_secret("sk-live-").is_none());
        assert!(detect_secret("just a normal string").is_none());
    }

    #[test]
    fn unused_variable_surfaced() {
        assert!(codes_resolved("let dead = 1\n").contains(&"tish-unused-variable"));
    }

    #[test]
    fn unused_import_surfaced() {
        let src = "import { a, b } from \"./m\"\nlet x = a\nx\n";
        assert!(codes_resolved(src).contains(&"tish-unused-import"));
    }

    #[test]
    fn unresolved_name_surfaced() {
        assert!(codes_resolved("let y = totallyMissing\ny\n").contains(&"tish-unresolved-name"));
    }

    #[test]
    fn clean_source_has_no_resolution_noise() {
        let out = codes_resolved("let x = 1\nlet y = x\ny\n");
        assert!(out.is_empty(), "expected clean, got {out:?}");
    }
}
