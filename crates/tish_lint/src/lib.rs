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
        // Block and the transparent comma-declarator group (#141: Multi was previously skipped).
        Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
            for st in statements {
                lint_stmt(st, out);
            }
        }
        // #140: control-flow CONDITION expressions are linted too, not just bodies.
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            lint_expr(cond, out);
            lint_stmt(then_branch, out);
            if let Some(e) = else_branch {
                lint_stmt(e, out);
            }
        }
        Statement::While { cond, body, .. } => {
            lint_expr(cond, out);
            lint_stmt(body, out);
        }
        Statement::ForOf { iterable, body, .. } => {
            lint_expr(iterable, out);
            lint_stmt(body, out);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                lint_stmt(i, out);
            }
            if let Some(c) = cond {
                lint_expr(c, out);
            }
            if let Some(u) = update {
                lint_expr(u, out);
            }
            lint_stmt(body, out);
        }
        Statement::FunDecl { body, .. } => lint_stmt(body, out),
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            lint_expr(expr, out);
            for (case_expr, stmts) in cases {
                if let Some(ce) = case_expr {
                    lint_expr(ce, out);
                }
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
        Statement::DoWhile { body, cond, .. } => {
            lint_stmt(body, out);
            lint_expr(cond, out);
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            tishlang_ast::ExportDeclaration::Named(inner) => lint_stmt(inner, out),
            // Walk `export default <expr>` too — its subtree was previously never linted, so e.g.
            // `export default { a: 1, a: 2 }` produced no tish-duplicate-key warning (#151).
            tishlang_ast::ExportDeclaration::Default(expr) => lint_expr(expr, out),
        },
        Statement::ExprStmt { expr, .. } => lint_expr(expr, out),
        Statement::VarDecl { init: Some(e), .. } => lint_expr(e, out),
        Statement::VarDeclDestructure { init, .. } => lint_expr(init, out),
        Statement::Return { value: Some(e), .. } => lint_expr(e, out),
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
        Expr::Object { props, .. } => {
            let mut seen: HashSet<&str> = HashSet::new();
            for p in props {
                if let ObjectProp::KeyValue(k, v, kspan) = p {
                    if !seen.insert(k.as_ref()) {
                        // Point at the duplicated KEY, not the enclosing `{` — so distinct dupes get
                        // distinct positions (#143).
                        out.push(LintDiagnostic {
                            code: "tish-duplicate-key",
                            message: format!("Duplicate object key `{}`", k),
                            line: kspan.start.0 as u32,
                            col: kspan.start.1 as u32,
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
        // #142: descend into a delete target (`delete obj[expr]` / `delete (expr).prop`).
        Expr::Delete { target, .. } => lint_expr(target, out),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Count `tish-duplicate-key` diagnostics the linter emits for `src` — a probe for whether the
    /// linter actually descends into a given position (the dup-key rule fires on any object literal
    /// the walk reaches).
    fn dup_keys(src: &str) -> usize {
        lint_source(src)
            .expect("parse")
            .iter()
            .filter(|d| d.code == "tish-duplicate-key")
            .count()
    }

    #[test]
    fn baseline_dup_key_in_plain_var_is_linted() {
        assert_eq!(dup_keys("let x = { a: 1, a: 2 }\n"), 1);
    }

    // #141: comma-separated declarators lower to Statement::Multi, which the linter skipped.
    #[test]
    fn lints_inside_comma_declarators() {
        assert!(
            dup_keys("let x = { a: 1, a: 2 }, y = 3\n") >= 1,
            "dup key in a comma-declarator (Statement::Multi) must be linted"
        );
    }

    // #140: control-flow CONDITION expressions were never linted (only bodies were walked).
    #[test]
    fn lints_inside_control_flow_conditions() {
        assert!(dup_keys("if ({ a: 1, a: 2 }) {}\n") >= 1, "if condition");
        assert!(dup_keys("while ({ a: 1, a: 2 }) { break }\n") >= 1, "while condition");
        assert!(
            dup_keys("for (let i = 0; ({ a: 1, a: 2 }); i = i + 1) { break }\n") >= 1,
            "for condition"
        );
        assert!(dup_keys("do { break } while ({ a: 1, a: 2 })\n") >= 1, "do-while condition");
        assert!(
            dup_keys("switch ({ a: 1, a: 2 }) { case 1: break }\n") >= 1,
            "switch discriminant"
        );
    }

    // #142: the linter never descended into a delete target.
    #[test]
    fn lints_inside_delete_target() {
        assert!(dup_keys("delete ({ a: 1, a: 2 }).a\n") >= 1, "delete target");
    }

    // #151: the `export default <expr>` subtree was never walked (only `export <named>` was).
    #[test]
    fn lints_inside_export_default() {
        assert!(
            dup_keys("export default { a: 1, a: 2 }\n") >= 1,
            "dup key inside `export default` must be linted"
        );
    }
}

#[cfg(test)]
mod dupkey_position_tests {
    use super::*;
    #[test]
    fn dup_key_points_at_the_duplicated_key() {
        // `let x = { a: 1, a: 2 }` — the duplicate `a` is at 1-indexed col 17; the object `{` is col 9.
        let d: Vec<_> = lint_source("let x = { a: 1, a: 2 }\n")
            .expect("parse")
            .into_iter()
            .filter(|d| d.code == "tish-duplicate-key")
            .collect();
        assert_eq!(d.len(), 1, "exactly one dup-key");
        assert_eq!((d[0].line, d[0].col), (1, 17), "#143: must point at the duplicated key, not the `{{`");
    }
}
