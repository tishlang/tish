//! AST optimization pass for Tish.
//!
//! Applies constant folding, short-circuit evaluation, conditional simplification,
//! and dead code elimination. Benefits all backends: bytecode VM, native, Rust codegen, JS transpilation.

use std::sync::Arc;

use tishlang_ast::{ArrowBody, BinOp, Expr, Literal, Program, Statement, UnaryOp};

/// Optimize a Tish program. Returns a new program with transformations applied.
pub fn optimize(program: &Program) -> Program {
    Program {
        statements: program.statements.iter().map(optimize_statement).collect(),
    }
}

fn optimize_statement(stmt: &Statement) -> Statement {
    match stmt {
        Statement::Block { statements, span } => {
            let optimized = optimize_block(statements);
            Statement::Block {
                statements: optimized,
                span: *span,
            }
        }
        Statement::Multi { statements, span } => Statement::Multi {
            statements: statements.iter().map(optimize_statement).collect(),
            span: *span,
        },
        Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann,
            init,
            span,
        } => Statement::VarDecl {
            name: Arc::clone(name),
            name_span: *name_span,
            mutable: *mutable,
            type_ann: type_ann.clone(),
            init: init.as_ref().map(optimize_expr),
            span: *span,
        },
        Statement::VarDeclDestructure {
            pattern,
            mutable,
            init,
            span,
        } => Statement::VarDeclDestructure {
            pattern: pattern.clone(),
            mutable: *mutable,
            init: optimize_expr(init),
            span: *span,
        },
        Statement::ExprStmt { expr, span } => Statement::ExprStmt {
            expr: optimize_expr(expr),
            span: *span,
        },
        Statement::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => {
            let opt_cond = optimize_expr(cond);
            // Conditional simplification: if cond is constant, take only the branch
            if let Expr::Literal { value, .. } = &opt_cond {
                let truthy = literal_is_truthy(value);
                return if truthy {
                    optimize_statement(then_branch)
                } else if let Some(else_b) = else_branch {
                    optimize_statement(else_b)
                } else {
                    Statement::Block {
                        statements: vec![],
                        span: *span,
                    }
                };
            }
            Statement::If {
                cond: opt_cond,
                then_branch: Box::new(optimize_statement(then_branch)),
                else_branch: else_branch
                    .as_ref()
                    .map(|b| Box::new(optimize_statement(b))),
                span: *span,
            }
        }
        Statement::While { cond, body, span } => Statement::While {
            cond: optimize_expr(cond),
            body: Box::new(optimize_statement(body)),
            span: *span,
        },
        Statement::For {
            init,
            cond,
            update,
            body,
            span,
        } => Statement::For {
            init: init.as_ref().map(|i| Box::new(optimize_statement(i))),
            cond: cond.as_ref().map(optimize_expr),
            update: update.as_ref().map(optimize_expr),
            body: Box::new(optimize_statement(body)),
            span: *span,
        },
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            span,
        } => Statement::ForOf {
            name: Arc::clone(name),
            name_span: *name_span,
            iterable: optimize_expr(iterable),
            body: Box::new(optimize_statement(body)),
            span: *span,
        },
        Statement::ForIn {
            name,
            name_span,
            object,
            body,
            span,
        } => Statement::ForIn {
            name: Arc::clone(name),
            name_span: *name_span,
            object: optimize_expr(object),
            body: Box::new(optimize_statement(body)),
            span: *span,
        },
        Statement::Return { value, span } => Statement::Return {
            value: value.as_ref().map(optimize_expr),
            span: *span,
        },
        Statement::Break { span } => Statement::Break { span: *span },
        Statement::Continue { span } => Statement::Continue { span: *span },
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
            name: Arc::clone(name),
            name_span: *name_span,
            params: params.clone(),
            rest_param: rest_param.clone(),
            return_type: return_type.clone(),
            body: Box::new(optimize_statement(body)),
            span: *span,
        },
        Statement::Switch {
            expr,
            cases,
            default_body,
            span,
        } => Statement::Switch {
            expr: optimize_expr(expr),
            cases: cases
                .iter()
                .map(|(ce, stmts)| (ce.as_ref().map(optimize_expr), optimize_block(stmts)))
                .collect(),
            default_body: default_body.as_ref().map(|stmts| optimize_block(stmts)),
            span: *span,
        },
        Statement::DoWhile { body, cond, span } => Statement::DoWhile {
            body: Box::new(optimize_statement(body)),
            cond: optimize_expr(cond),
            span: *span,
        },
        Statement::Throw { value, span } => Statement::Throw {
            value: optimize_expr(value),
            span: *span,
        },
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            span,
        } => Statement::Try {
            body: Box::new(optimize_statement(body)),
            catch_param: catch_param.clone(),
            catch_param_span: *catch_param_span,
            catch_body: catch_body.as_ref().map(|b| Box::new(optimize_statement(b))),
            finally_body: finally_body
                .as_ref()
                .map(|b| Box::new(optimize_statement(b))),
            span: *span,
        },
        Statement::Import { .. }
        | Statement::Export { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. }
        | Statement::DeclareFun { .. } => stmt.clone(),
    }
}

/// Optimize block with dead code elimination: remove statements after return/throw.
fn optimize_block(statements: &[Statement]) -> Vec<Statement> {
    let mut result = Vec::new();
    for stmt in statements {
        if let Some(last) = result.last() {
            if stmt_always_returns_or_throws(last) {
                // Dead code - skip
                continue;
            }
        }
        result.push(optimize_statement(stmt));
    }
    result
}

fn stmt_always_returns_or_throws(stmt: &Statement) -> bool {
    match stmt {
        Statement::Return { .. } | Statement::Throw { .. } => true,
        Statement::If {
            cond: Expr::Literal { value, .. },
            then_branch,
            else_branch,
            ..
        } => {
            let truthy = literal_is_truthy(value);
            if truthy {
                stmt_always_returns_or_throws(then_branch)
            } else if let Some(else_b) = else_branch {
                stmt_always_returns_or_throws(else_b)
            } else {
                false
            }
        }
        Statement::If { .. } => false,
        _ => false,
    }
}

fn optimize_expr(expr: &Expr) -> Expr {
    match expr {
        Expr::Literal { value, span } => Expr::Literal {
            value: value.clone(),
            span: *span,
        },
        Expr::Ident { name, span } => Expr::Ident {
            name: Arc::clone(name),
            span: *span,
        },
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => {
            let opt_left = optimize_expr(left);
            let opt_right = optimize_expr(right);

            // Short-circuit for And/Or when left is constant
            if *op == BinOp::And {
                if let Expr::Literal { value, .. } = &opt_left {
                    return if literal_is_truthy(value) {
                        opt_right
                    } else {
                        Expr::Literal {
                            value: Literal::Bool(false),
                            span: *span,
                        }
                    };
                }
            }
            if *op == BinOp::Or {
                if let Expr::Literal { value, .. } = &opt_left {
                    return if literal_is_truthy(value) {
                        Expr::Literal {
                            value: Literal::Bool(true),
                            span: *span,
                        }
                    } else {
                        opt_right
                    };
                }
            }

            // Constant folding when both are literals
            if let (Expr::Literal { value: lv, .. }, Expr::Literal { value: rv, .. }) =
                (&opt_left, &opt_right)
            {
                if let Some(folded) = try_fold_binop(lv, *op, rv) {
                    return Expr::Literal {
                        value: folded,
                        span: *span,
                    };
                }
            }

            // A5: Algebraic simplification (x+0=x, x*1=x, etc.).
            // Applied after constant folding so e.g. x*(1+0) → x*1 → x.
            if let Some(simplified) = try_algebraic_simplify(*op, &opt_left, &opt_right, *span) {
                return simplified;
            }

            Expr::Binary {
                left: Box::new(opt_left),
                op: *op,
                right: Box::new(opt_right),
                span: *span,
            }
        }
        Expr::Unary { op, operand, span } => {
            let opt_operand = optimize_expr(operand);
            if let Expr::Literal { value, .. } = &opt_operand {
                if let Some(folded) = try_fold_unary(*op, value) {
                    return Expr::Literal {
                        value: folded,
                        span: *span,
                    };
                }
            }
            Expr::Unary {
                op: *op,
                operand: Box::new(opt_operand),
                span: *span,
            }
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            span,
        } => {
            let opt_cond = optimize_expr(cond);
            if let Expr::Literal { value, .. } = &opt_cond {
                return if literal_is_truthy(value) {
                    optimize_expr(then_branch)
                } else {
                    optimize_expr(else_branch)
                };
            }
            Expr::Conditional {
                cond: Box::new(opt_cond),
                then_branch: Box::new(optimize_expr(then_branch)),
                else_branch: Box::new(optimize_expr(else_branch)),
                span: *span,
            }
        }
        Expr::Call { callee, args, span } => Expr::Call {
            callee: Box::new(optimize_expr(callee)),
            args: args
                .iter()
                .map(|a| match a {
                    tishlang_ast::CallArg::Expr(e) => tishlang_ast::CallArg::Expr(optimize_expr(e)),
                    tishlang_ast::CallArg::Spread(e) => {
                        tishlang_ast::CallArg::Spread(optimize_expr(e))
                    }
                })
                .collect(),
            span: *span,
        },
        Expr::New { callee, args, span } => Expr::New {
            callee: Box::new(optimize_expr(callee)),
            args: args
                .iter()
                .map(|a| match a {
                    tishlang_ast::CallArg::Expr(e) => tishlang_ast::CallArg::Expr(optimize_expr(e)),
                    tishlang_ast::CallArg::Spread(e) => {
                        tishlang_ast::CallArg::Spread(optimize_expr(e))
                    }
                })
                .collect(),
            span: *span,
        },
        Expr::Member {
            object,
            prop,
            optional,
            span,
        } => {
            let opt_obj = optimize_expr(object);
            let opt_prop = match prop {
                tishlang_ast::MemberProp::Name { name, span } => tishlang_ast::MemberProp::Name {
                    name: Arc::clone(name),
                    span: *span,
                },
                tishlang_ast::MemberProp::Expr(e) => {
                    tishlang_ast::MemberProp::Expr(Box::new(optimize_expr(e)))
                }
            };
            Expr::Member {
                object: Box::new(opt_obj),
                prop: opt_prop,
                optional: *optional,
                span: *span,
            }
        }
        Expr::Index {
            object,
            index,
            optional,
            span,
        } => Expr::Index {
            object: Box::new(optimize_expr(object)),
            index: Box::new(optimize_expr(index)),
            optional: *optional,
            span: *span,
        },
        Expr::NullishCoalesce { left, right, span } => {
            let opt_left = optimize_expr(left);
            if let Expr::Literal {
                value: Literal::Null,
                ..
            } = &opt_left
            {
                return optimize_expr(right);
            }
            Expr::NullishCoalesce {
                left: Box::new(opt_left),
                right: Box::new(optimize_expr(right)),
                span: *span,
            }
        }
        Expr::Array { elements, span } => Expr::Array {
            elements: elements
                .iter()
                .map(|e| match e {
                    tishlang_ast::ArrayElement::Expr(ex) => {
                        tishlang_ast::ArrayElement::Expr(optimize_expr(ex))
                    }
                    tishlang_ast::ArrayElement::Spread(ex) => {
                        tishlang_ast::ArrayElement::Spread(optimize_expr(ex))
                    }
                })
                .collect(),
            span: *span,
        },
        Expr::Object { props, span } => Expr::Object {
            props: props
                .iter()
                .map(|p| match p {
                    tishlang_ast::ObjectProp::KeyValue(k, v, s) => {
                        tishlang_ast::ObjectProp::KeyValue(Arc::clone(k), optimize_expr(v), *s)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        tishlang_ast::ObjectProp::Spread(optimize_expr(e))
                    }
                })
                .collect(),
            span: *span,
        },
        Expr::Assign { name, value, span } => Expr::Assign {
            name: Arc::clone(name),
            value: Box::new(optimize_expr(value)),
            span: *span,
        },
        Expr::TypeOf { operand, span } => Expr::TypeOf {
            operand: Box::new(optimize_expr(operand)),
            span: *span,
        },
        Expr::Delete { target, span } => Expr::Delete {
            target: Box::new(optimize_expr(target)),
            span: *span,
        },
        Expr::PostfixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixInc { .. }
        | Expr::PrefixDec { .. } => expr.clone(),
        Expr::CompoundAssign {
            name,
            op,
            value,
            span,
        } => Expr::CompoundAssign {
            name: Arc::clone(name),
            op: *op,
            value: Box::new(optimize_expr(value)),
            span: *span,
        },
        Expr::LogicalAssign {
            name,
            op,
            value,
            span,
        } => Expr::LogicalAssign {
            name: Arc::clone(name),
            op: *op,
            value: Box::new(optimize_expr(value)),
            span: *span,
        },
        Expr::MemberAssign {
            object,
            prop,
            value,
            span,
        } => Expr::MemberAssign {
            object: Box::new(optimize_expr(object)),
            prop: Arc::clone(prop),
            value: Box::new(optimize_expr(value)),
            span: *span,
        },
        Expr::IndexAssign {
            object,
            index,
            value,
            span,
        } => Expr::IndexAssign {
            object: Box::new(optimize_expr(object)),
            index: Box::new(optimize_expr(index)),
            value: Box::new(optimize_expr(value)),
            span: *span,
        },
        Expr::ArrowFunction {
            async_,
            params,
            body,
            span,
        } => {
            let opt_body = match body {
                ArrowBody::Expr(e) => ArrowBody::Expr(Box::new(optimize_expr(e))),
                ArrowBody::Block(s) => ArrowBody::Block(Box::new(optimize_statement(s))),
            };
            Expr::ArrowFunction {
                async_: *async_,
                params: params.clone(),
                body: opt_body,
                span: *span,
            }
        }
        Expr::TemplateLiteral {
            quasis,
            exprs,
            span,
        } => Expr::TemplateLiteral {
            quasis: quasis.iter().map(Arc::clone).collect(),
            exprs: exprs.iter().map(optimize_expr).collect(),
            span: *span,
        },
        Expr::Await { operand, span } => Expr::Await {
            operand: Box::new(optimize_expr(operand)),
            span: *span,
        },
        Expr::JsxElement { .. } | Expr::JsxFragment { .. } => expr.clone(),
        Expr::NativeModuleLoad {
            spec,
            export_name,
            span,
        } => Expr::NativeModuleLoad {
            spec: Arc::clone(spec),
            export_name: Arc::clone(export_name),
            span: *span,
        },
    }
}

fn literal_is_truthy(lit: &Literal) -> bool {
    match lit {
        Literal::Null => false,
        Literal::Bool(b) => *b,
        Literal::Number(n) => *n != 0.0 && !n.is_nan(),
        Literal::String(s) => !s.is_empty(),
    }
}

fn literal_strict_eq(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Number(x), Literal::Number(y)) => {
            if x.is_nan() || y.is_nan() {
                false
            } else {
                x == y
            }
        }
        (Literal::String(x), Literal::String(y)) => x == y,
        (Literal::Bool(x), Literal::Bool(y)) => x == y,
        (Literal::Null, Literal::Null) => true,
        _ => false,
    }
}

/// JS `Number.prototype.toString` (radix 10). **Kept byte-for-byte in sync with
/// `tishlang_core::js_number_to_string`** so a constant-folded `"" + n` here matches the
/// runtime conversion there; `tish_opt` deliberately does not depend on `tish_core` (it is a
/// lean AST pass), hence the small duplication of this fixed-spec algorithm. See that function
/// for the full commentary.
fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == f64::INFINITY {
        return "Infinity".to_string();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if value == 0.0 {
        // ECMAScript `Number::toString`: both `+0` and `-0` → `"0"` (a constant-folded
        // `"" + (-0)` must match the runtime ToString, not the inspect form). (#247)
        return "0".to_string();
    }
    let negative = value < 0.0;
    let sci = format!("{:e}", value.abs());
    let (mantissa, exp_str) = sci
        .split_once('e')
        .expect("LowerExp formatting always contains 'e'");
    let exp: i32 = exp_str
        .parse()
        .expect("LowerExp exponent is a valid integer");
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let k = digits.len() as i32;
    let point = exp + 1;

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if k <= point && point <= 21 {
        out.push_str(&digits);
        out.push_str(&"0".repeat((point - k) as usize));
    } else if 0 < point && point <= 21 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-point) as usize));
        out.push_str(&digits);
    } else {
        let e = point - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e >= 0 { '+' } else { '-' });
        out.push_str(&e.abs().to_string());
    }
    out
}

fn literal_to_display_string(lit: &Literal) -> String {
    match lit {
        // Must match the runtime exactly so constant-folded `"" + n` agrees with the
        // unfolded path (see `js_number_to_string` below).
        Literal::Number(n) => js_number_to_string(*n),
        Literal::String(s) => s.to_string(),
        Literal::Bool(b) => b.to_string(),
        Literal::Null => "null".to_string(),
    }
}

fn literal_as_number(lit: &Literal) -> f64 {
    match lit {
        Literal::Number(n) => *n,
        Literal::Bool(true) => 1.0,
        Literal::Bool(false) => 0.0,
        Literal::Null => 0.0,
        Literal::String(s) => s.parse().unwrap_or(f64::NAN),
    }
}

/// Algebraic simplification: x+0→x, x*1→x, etc.
/// Only applies when the literal is a clean 0 or 1 (no NaN/Inf).
fn try_algebraic_simplify(
    op: BinOp,
    left: &Expr,
    right: &Expr,
    span: tishlang_ast::Span,
) -> Option<Expr> {
    use BinOp::*;
    fn num_is_zero(n: f64) -> bool {
        n == 0.0 && !n.is_nan() && n.is_finite()
    }
    fn num_is_one(n: f64) -> bool {
        (n - 1.0).abs() < f64::EPSILON && !n.is_nan() && n.is_finite()
    }

    match op {
        Add => {
            if let Expr::Literal {
                value: Literal::Number(r),
                ..
            } = right
            {
                if num_is_zero(*r) {
                    return Some(left.clone());
                }
            }
            if let Expr::Literal {
                value: Literal::Number(l),
                ..
            } = left
            {
                if num_is_zero(*l) {
                    return Some(right.clone());
                }
            }
        }
        Sub => {
            if let Expr::Literal {
                value: Literal::Number(r),
                ..
            } = right
            {
                if num_is_zero(*r) {
                    return Some(left.clone());
                }
            }
        }
        Mul => {
            if let Expr::Literal {
                value: Literal::Number(r),
                ..
            } = right
            {
                if num_is_one(*r) {
                    return Some(left.clone());
                }
                if num_is_zero(*r) {
                    return Some(Expr::Literal {
                        value: Literal::Number(0.0),
                        span,
                    });
                }
            }
            if let Expr::Literal {
                value: Literal::Number(l),
                ..
            } = left
            {
                if num_is_one(*l) {
                    return Some(right.clone());
                }
                if num_is_zero(*l) {
                    return Some(Expr::Literal {
                        value: Literal::Number(0.0),
                        span,
                    });
                }
            }
        }
        Div => {
            if let Expr::Literal {
                value: Literal::Number(r),
                ..
            } = right
            {
                if num_is_one(*r) {
                    return Some(left.clone());
                }
            }
        }
        Pow => {
            if let Expr::Literal {
                value: Literal::Number(r),
                ..
            } = right
            {
                if num_is_one(*r) {
                    return Some(left.clone());
                }
                if num_is_zero(*r) {
                    return Some(Expr::Literal {
                        value: Literal::Number(1.0),
                        span,
                    });
                }
            }
            if let Expr::Literal {
                value: Literal::Number(l),
                ..
            } = left
            {
                if num_is_one(*l) {
                    return Some(Expr::Literal {
                        value: Literal::Number(1.0),
                        span,
                    });
                }
            }
        }
        _ => {}
    }
    None
}

// JS ToInt32/ToUint32 for the constant folder. NaN/±Infinity → 0 (`f64 as i64` saturates, so a
// folded `(1e308 * 10) | 0` would otherwise give -1 while the runtime gives 0). `tish_opt` has no
// `tish_core` dep, so these mirror `tishlang_core::to_int32`/`to_uint32` (kept in sync, pure spec).
#[inline]
fn fold_to_int32(x: f64) -> i32 {
    if x.is_finite() {
        x as i64 as i32
    } else {
        0
    }
}
#[inline]
fn fold_to_uint32(x: f64) -> u32 {
    if x.is_finite() {
        x as i64 as u32
    } else {
        0
    }
}

/// Constant-fold a relational comparison (`<` `<=` `>` `>=`). Two string literals
/// compare lexicographically; otherwise the numeric coercions `ln`/`rn` are used.
/// `pred` maps the `Ordering` to a bool; a NaN-involved numeric comparison has no
/// ordering and is `false` — matching the VM/interp/native runtime exactly.
fn fold_relational<F>(left: &Literal, right: &Literal, ln: f64, rn: f64, pred: F) -> bool
where
    F: FnOnce(std::cmp::Ordering) -> bool,
{
    let ord = match (left, right) {
        (Literal::String(a), Literal::String(b)) => Some(a.as_ref().cmp(b.as_ref())),
        _ => ln.partial_cmp(&rn),
    };
    ord.map(pred).unwrap_or(false)
}

fn try_fold_binop(left: &Literal, op: BinOp, right: &Literal) -> Option<Literal> {
    use BinOp::*;
    let ln = literal_as_number(left);
    let rn = literal_as_number(right);

    let result = match op {
        Add => {
            if matches!(left, Literal::String(_)) || matches!(right, Literal::String(_)) {
                return Some(Literal::String(
                    format!(
                        "{}{}",
                        literal_to_display_string(left),
                        literal_to_display_string(right)
                    )
                    .into(),
                ));
            }
            Literal::Number(ln + rn)
        }
        Sub => Literal::Number(ln - rn),
        Mul => Literal::Number(ln * rn),
        // IEEE division/remainder, matching JS + the VM's `eval_binop` + interp + rust-AOT:
        // `5/0` → Infinity, `-5/0` → -Infinity, `0/0` → NaN, `5%0` → NaN. The former
        // `if rn == 0.0 { NaN }` folded `5/0` to NaN at compile time, diverging from every runtime
        // path (which all produce Infinity) — a constant-fold-vs-runtime inconsistency.
        Div => Literal::Number(ln / rn),
        Mod => Literal::Number(ln % rn),
        Pow => Literal::Number(ln.powf(rn)),
        Eq => Literal::Bool(literal_strict_eq(left, right)),
        Ne => Literal::Bool(!literal_strict_eq(left, right)),
        StrictEq => Literal::Bool(literal_strict_eq(left, right)),
        StrictNe => Literal::Bool(!literal_strict_eq(left, right)),
        // Relational ops fold lexicographically when BOTH operands are string
        // literals (JS semantics — must match the VM/interp/native runtime), else
        // numerically. A NaN-involved numeric comparison is always false.
        Lt => Literal::Bool(fold_relational(left, right, ln, rn, |o| o.is_lt())),
        Le => Literal::Bool(fold_relational(left, right, ln, rn, |o| o.is_le())),
        Gt => Literal::Bool(fold_relational(left, right, ln, rn, |o| o.is_gt())),
        Ge => Literal::Bool(fold_relational(left, right, ln, rn, |o| o.is_ge())),
        And => Literal::Bool(literal_is_truthy(left) && literal_is_truthy(right)),
        Or => Literal::Bool(literal_is_truthy(left) || literal_is_truthy(right)),
        // ToInt32/ToUint32 (modulo 2³², NaN/±Infinity → 0), matching the VM/interp exactly.
        BitAnd => Literal::Number((fold_to_int32(ln) & fold_to_int32(rn)) as f64),
        BitOr => Literal::Number((fold_to_int32(ln) | fold_to_int32(rn)) as f64),
        BitXor => Literal::Number((fold_to_int32(ln) ^ fold_to_int32(rn)) as f64),
        Shl => Literal::Number(fold_to_int32(ln).wrapping_shl(fold_to_uint32(rn)) as f64),
        Shr => Literal::Number(fold_to_int32(ln).wrapping_shr(fold_to_uint32(rn)) as f64),
        UShr => Literal::Number(fold_to_uint32(ln).wrapping_shr(fold_to_uint32(rn)) as f64),
        In => return None, // Requires object/array on right
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_from_source(src: &str) -> Program {
        tishlang_parser::parse(src).expect("parse")
    }

    fn has_literal_number(expr: &Expr, n: f64) -> bool {
        if let Expr::Literal {
            value: Literal::Number(x),
            ..
        } = expr
        {
            (*x - n).abs() < f64::EPSILON
        } else {
            false
        }
    }

    #[test]
    fn constant_fold_add() {
        let program = program_from_source("1 + 2");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(has_literal_number(expr, 3.0), "expected 3, got {:?}", expr);
    }

    #[test]
    fn constant_fold_unary_neg() {
        let program = program_from_source("-42");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            has_literal_number(expr, -42.0),
            "expected -42, got {:?}",
            expr
        );
    }

    #[test]
    fn short_circuit_false_and() {
        let program = program_from_source("false && foo");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            matches!(
                expr,
                Expr::Literal {
                    value: Literal::Bool(false),
                    ..
                }
            ),
            "expected false, got {:?}",
            expr
        );
    }

    #[test]
    fn conditional_simplify_true() {
        let program = program_from_source("true ? 1 : 2");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(has_literal_number(expr, 1.0), "expected 1, got {:?}", expr);
    }

    #[test]
    fn algebraic_simplify_x_plus_zero() {
        // x + 0 → x (after constant fold, 0 is literal)
        let program = program_from_source("x + 0");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            matches!(expr, Expr::Ident { name, .. } if name.as_ref() == "x"),
            "expected Ident(x), got {:?}",
            expr
        );
    }

    #[test]
    fn algebraic_simplify_x_times_one() {
        let program = program_from_source("x * 1");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            matches!(expr, Expr::Ident { name, .. } if name.as_ref() == "x"),
            "expected Ident(x), got {:?}",
            expr
        );
    }

    #[test]
    fn algebraic_simplify_chain() {
        // x * (1 + 0) → constant fold 1+0=1 → x*1 → x
        let program = program_from_source("x * (1 + 0)");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            matches!(expr, Expr::Ident { name, .. } if name.as_ref() == "x"),
            "expected Ident(x) after x*(1+0) → x*1 → x, got {:?}",
            expr
        );
    }

    #[test]
    fn algebraic_simplify_pow_one() {
        let program = program_from_source("x ** 1");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            matches!(expr, Expr::Ident { name, .. } if name.as_ref() == "x"),
            "expected Ident(x), got {:?}",
            expr
        );
    }

    #[test]
    fn algebraic_simplify_pow_zero() {
        let program = program_from_source("x ** 0");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(has_literal_number(expr, 1.0), "expected 1, got {:?}", expr);
    }

    #[test]
    fn algebraic_simplify_one_pow_x() {
        let program = program_from_source("1 ** x");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(has_literal_number(expr, 1.0), "expected 1, got {:?}", expr);
    }

    #[test]
    fn nullish_coalesce_null_simplify() {
        let program = program_from_source("null ?? x");
        let opt = optimize(&program);
        let expr = match &opt.statements[..] {
            [tishlang_ast::Statement::ExprStmt { expr, .. }] => expr,
            _ => panic!("expected single expr stmt"),
        };
        assert!(
            matches!(expr, Expr::Ident { name, .. } if name.as_ref() == "x"),
            "expected Ident(x), got {:?}",
            expr
        );
    }
}

fn try_fold_unary(op: UnaryOp, operand: &Literal) -> Option<Literal> {
    use UnaryOp::*;
    let result = match op {
        Not => Literal::Bool(!literal_is_truthy(operand)),
        Neg => Literal::Number(-literal_as_number(operand)),
        Pos => Literal::Number(literal_as_number(operand)),
        BitNot => Literal::Number(!fold_to_int32(literal_as_number(operand)) as f64),
        Void => Literal::Null,
    };
    Some(result)
}
