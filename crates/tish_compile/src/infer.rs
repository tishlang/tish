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

use std::collections::{HashMap, HashSet};
use tishlang_ast::{
    ArrowBody, BinOp, CallArg, Expr, FunParam, Literal, Program, Statement, TypeAnnotation,
};

/// Scoped type environment used during inference.
#[derive(Default, Clone)]
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

fn is_string(ann: &TypeAnnotation) -> bool {
    matches!(ann, TypeAnnotation::Simple(s) if s.as_ref() == "string")
}

fn is_bool(ann: &TypeAnnotation) -> bool {
    matches!(ann, TypeAnnotation::Simple(s) if s.as_ref() == "boolean")
}

/// Element type of an array literal of uniform native scalars (number/string/boolean), for
/// `let xs = [1, 2, 3]` -> `number[]`. Bails (None) on empty, spread, mixed, or non-scalar
/// elements so the binding stays a boxed array.
fn infer_array_elem(elements: &[tishlang_ast::ArrayElement], ctx: &InferCtx) -> Option<TypeAnnotation> {
    use tishlang_ast::ArrayElement;
    if elements.is_empty() {
        return None;
    }
    let mut elem: Option<TypeAnnotation> = None;
    for el in elements {
        let e = match el {
            ArrayElement::Expr(e) => e,
            ArrayElement::Spread(_) => return None,
        };
        let t = infer_expr_type(e, ctx)?;
        if !(is_number(&t) || is_string(&t) || is_bool(&t)) {
            return None;
        }
        match &elem {
            None => elem = Some(t),
            Some(prev) if prev != &t => return None,
            _ => {}
        }
    }
    elem
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
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
                    // Bitwise/shift coerce to int32 and yield a Number.
                    | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
                    | BinOp::Shl | BinOp::Shr | BinOp::UShr => Some(number_ann()),
                    BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::StrictEq
                    | BinOp::StrictNe => Some(bool_ann()),
                    _ => None,
                }
            } else if is_string(&lt) && is_string(&rt) {
                // M2: `string + string` concatenates → string; `===`/`!==` → boolean. Relational
                // comparisons stay boxed (UTF-16 vs UTF-8 ordering differs outside the BMP).
                match op {
                    BinOp::Add => Some(string_ann()),
                    BinOp::StrictEq | BinOp::StrictNe => Some(bool_ann()),
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
                // `~x` is a Number.
                UnaryOp::BitNot => {
                    let t = infer_expr_type(operand, ctx)?;
                    is_number(&t).then(number_ann)
                }
                _ => None,
            }
        }
        // Index of a typed array yields its element type (`a[i]` where `a: T[]` → `T`).
        Expr::Index { object, .. } => match infer_expr_type(object, ctx) {
            Some(TypeAnnotation::Array(elem)) => Some(*elem),
            _ => None,
        },
        _ => None,
    }
}

/// Run inference over a program, returning a modified Program with additional
/// type annotations filled in on `VarDecl` nodes.
pub fn infer_program(program: &Program) -> Program {
    // M4 (opt-in via TISH_PARAM_INFER) runs FIRST: give unannotated params used PURELY
    // numerically a synthetic `: number`. Doing this *before* local/struct inference is what
    // lets derived numeric locals (`let x0 = (px / w) * 3`) be proven numeric off the now-known
    // param types — otherwise they fall back to boxed `Value` and the whole hot loop boxes with
    // them (the difference between an idiomatic numeric fn going native vs staying boxed).
    // Conservative — any non-numeric / write / escape use bails (param stays boxed Value).
    let p = if std::env::var("TISH_PARAM_INFER").map(|v| v != "0").unwrap_or(false) {
        param_infer_program(program.clone())
    } else {
        program.clone()
    };
    // Base + local-numeric inference (annotates `let` nodes), now seeing M4's param types.
    let mut ctx = InferCtx::new();
    let p = Program {
        statements: infer_statements(&p.statements, &mut ctx),
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
// M4: parameter type inference (conservative, sound, opt-in)
// ---------------------------------------------------------------------------

fn param_infer_program(program: Program) -> Program {
    Program {
        statements: program.statements.into_iter().map(pi_stmt).collect(),
    }
}

fn pi_stmt(s: Statement) -> Statement {
    if let Statement::FunDecl {
        async_,
        name,
        name_span,
        params,
        rest_param,
        return_type,
        body,
        span,
    } = s
    {
        // Locals provably numeric in this body (annotated, incl. base-inferred `let i = 0`), so a
        // bare loop counter `i` counts as a numeric operand and `i < n` can prove the param `n`.
        let mut nums = HashSet::new();
        collect_numeric_locals(&body, &mut nums);
        let new_params = params
            .into_iter()
            .map(|p| match p {
                FunParam::Simple(mut tp) => {
                    if tp.type_ann.is_none()
                        && tp.default.is_none()
                        && nus_stmt(&body, tp.name.as_ref(), &nums)
                    {
                        tp.type_ann = Some(TypeAnnotation::Simple(std::sync::Arc::from("number")));
                    }
                    FunParam::Simple(tp)
                }
                other => other,
            })
            .collect();
        Statement::FunDecl {
            async_,
            name,
            name_span,
            params: new_params,
            rest_param,
            return_type,
            body,
            span,
        }
    } else {
        s
    }
}

/// Names of locals in `s` annotated (or base-inferred) `: number`. Consulted by `numeric_provable`
/// so a bare numeric local (e.g. a `let i: number` loop counter) counts as a numeric operand —
/// letting `i < n` / `i * n + k` prove the *param* `n` numeric. Flat across nested scopes; at worst
/// it over-includes a shadowed name, which only widens inference (still bounded by the same
/// caller-passes-a-number assumption M4 already makes).
fn collect_numeric_locals(s: &Statement, out: &mut HashSet<String>) {
    use Statement::*;
    match s {
        VarDecl {
            name,
            type_ann,
            init,
            ..
        } => {
            // Annotated `: number`, OR **base-inferred** from a numeric-literal initializer
            // (`let i = 0`, `let x = 0.0`) — the common loop-counter / accumulator pattern. A
            // numeric literal is unambiguously a number at init; this is what lets `i < n` prove
            // the *param* `n` numeric. Over-inclusion (if the local is later reassigned to a
            // non-number) only *widens* param inference, still bounded by M4's
            // caller-passes-a-number / NaN-coercion soundness and the corpus/gauntlet guards.
            let numeric = type_ann.as_ref().is_some_and(is_number)
                || matches!(
                    init,
                    Some(tishlang_ast::Expr::Literal {
                        value: tishlang_ast::Literal::Number(_),
                        ..
                    })
                );
            if numeric {
                out.insert(name.to_string());
            }
        }
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().for_each(|x| collect_numeric_locals(x, out))
        }
        If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_numeric_locals(then_branch, out);
            if let Some(e) = else_branch {
                collect_numeric_locals(e, out);
            }
        }
        For { init, body, .. } => {
            if let Some(i) = init {
                collect_numeric_locals(i, out);
            }
            collect_numeric_locals(body, out);
        }
        While { body, .. } => collect_numeric_locals(body, out),
        _ => {}
    }
}

/// One side of an OVERLOADED binop (`+`, comparisons). If `operand` is bare `name`, the `other`
/// side must be PROVABLY numeric (else `name + x` / `name < x` could be string ops, and `name`
/// a string). If `operand` is a sub-expr, recurse (its own context decides).
fn nus_overloaded(operand: &Expr, other: &Expr, name: &str, nums: &HashSet<String>) -> bool {
    if matches!(operand, Expr::Ident { name: n, .. } if n.as_ref() == name) {
        return numeric_provable(other, nums);
    }
    nus_expr(operand, name, nums)
}

/// `e` is PROVABLY a number: a number literal, arithmetic (`-`/`*`/`/`/`%`/`**`), numeric unary,
/// or a Math intrinsic. Bare variables and `+`/comparisons are NOT provable (could be strings).
fn numeric_provable(e: &Expr, nums: &HashSet<String>) -> bool {
    use Expr::*;
    match e {
        Literal {
            value: tishlang_ast::Literal::Number(_),
            ..
        } => true,
        // A local proven numeric in this function (annotated `: number`, incl. base-inferred
        // `let i = 0`) — so `i` as the OTHER operand of `i < n` / `i * n` proves `n` numeric.
        Ident { name: n, .. } => nums.contains(n.as_ref()),
        Binary {
            left, op, right, ..
        } => {
            use tishlang_ast::BinOp::*;
            matches!(
                op,
                Sub | Mul | Div | Mod | Pow | BitAnd | BitOr | BitXor | Shl | Shr | UShr
            ) && numeric_provable(left, nums)
                && numeric_provable(right, nums)
        }
        Unary { op, operand, .. } => {
            matches!(
                op,
                tishlang_ast::UnaryOp::Neg | tishlang_ast::UnaryOp::Pos | tishlang_ast::UnaryOp::BitNot
            ) && numeric_provable(operand, nums)
        }
        Call { callee, .. } => matches!(callee.as_ref(),
            Expr::Member { object, prop: tishlang_ast::MemberProp::Name { name: m, .. }, .. }
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                    && matches!(m.as_ref(),
                        "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" | "exp" | "trunc" | "log")),
        _ => false,
    }
}

/// Every use of `name` within `s` is a numeric-operand use (so `name` can lower to `f64`).
fn nus_stmt(s: &Statement, name: &str, nums: &HashSet<String>) -> bool {
    use Statement::*;
    match s {
        Block { statements, .. } => statements.iter().all(|x| nus_stmt(x, name, nums)),
        Return { value, .. } => value.as_ref().is_none_or(|e| nus_num_operand(e, name, nums)),
        If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            nus_expr(cond, name, nums)
                && nus_stmt(then_branch, name, nums)
                && else_branch.as_ref().is_none_or(|e| nus_stmt(e, name, nums))
        }
        ExprStmt { expr, .. } => nus_expr(expr, name, nums),
        While { cond, body, .. } => nus_expr(cond, name, nums) && nus_stmt(body, name, nums),
        For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_none_or(|x| nus_stmt(x, name, nums))
                && cond.as_ref().is_none_or(|e| nus_expr(e, name, nums))
                && update.as_ref().is_none_or(|e| nus_expr(e, name, nums))
                && nus_stmt(body, name, nums)
        }
        VarDecl {
            name: vn, init, ..
        } => vn.as_ref() != name && init.as_ref().is_none_or(|e| nus_expr(e, name, nums)),
        Break { .. } | Continue { .. } => true,
        // Any other statement (switch/throw/try/nested fn/...) -> bail (don't infer this param).
        _ => false,
    }
}

/// `e` with `name` used only as a numeric operand. A bare `Ident(name)` at THIS level is not a
/// numeric operand (only valid inside a numeric parent), so it returns false here.
fn nus_expr(e: &Expr, name: &str, nums: &HashSet<String>) -> bool {
    use Expr::*;
    match e {
        Literal { .. } => true,
        Ident { name: n, .. } => n.as_ref() != name,
        Binary {
            left, op, right, ..
        } => {
            use tishlang_ast::BinOp::*;
            match op {
                // Unambiguously numeric — `name` as either operand is definitely a number.
                // Includes the bitwise/shift family: JS coerces both sides to int32, so
                // `name & x` / `name >>> x` proves `name` numeric just like `name * x`.
                Sub | Mul | Div | Mod | Pow | BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                    nus_num_operand(left, name, nums) && nus_num_operand(right, name, nums)
                }
                // OVERLOADED: `+` is also string concat, `<`/`===` also compare strings. If
                // `name` is a DIRECT operand here, the OTHER side must be PROVABLY numeric to
                // conclude `name` is a number — this is what stops `first + ":"` typing `first`.
                Add | Lt | Le | Gt | Ge | StrictEq | StrictNe => {
                    nus_overloaded(left, right, name, nums)
                        && nus_overloaded(right, left, name, nums)
                }
                // Logical `&&`/`||`: recurse — a param used numerically *inside* a condition
                // operand (e.g. `iter < maxIter && x*x + y*y <= 4`) is a numeric use; a bare
                // `name && x` is not (the recursion's `Ident(name)` case returns false).
                And | Or => nus_expr(left, name, nums) && nus_expr(right, name, nums),
                // Anything else (`Eq`/`Ne`/`In`): `name` must be absent.
                _ => !pi_mentions(left, name) && !pi_mentions(right, name),
            }
        }
        Unary { op, operand, .. } => {
            if matches!(
                op,
                tishlang_ast::UnaryOp::Neg | tishlang_ast::UnaryOp::Pos | tishlang_ast::UnaryOp::BitNot
            ) {
                nus_num_operand(operand, name, nums)
            } else {
                !pi_mentions(operand, name)
            }
        }
        Index { object, index, .. } => {
            !pi_mentions(object, name) && nus_num_operand(index, name, nums)
        }
        Call { callee, args, .. } => {
            !pi_mentions(callee, name)
                && args.iter().all(|a| match a {
                    tishlang_ast::CallArg::Expr(x) => nus_arg(x, name, nums),
                    tishlang_ast::CallArg::Spread(_) => false,
                })
        }
        Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            nus_expr(cond, name, nums)
                && nus_num_operand(then_branch, name, nums)
                && nus_num_operand(else_branch, name, nums)
        }
        // Assignment to a DIFFERENT var, where the RHS may use `name` numerically (e.g.
        // `sum = sum + a[i*N+k]`). Writing to `name` itself bails (its type could change).
        Assign { name: an, value, .. }
        | CompoundAssign { name: an, value, .. }
        | LogicalAssign { name: an, value, .. } => {
            an.as_ref() != name && nus_expr(value, name, nums)
        }
        PostfixInc { name: n, .. }
        | PostfixDec { name: n, .. }
        | PrefixInc { name: n, .. }
        | PrefixDec { name: n, .. } => n.as_ref() != name,
        // `c[i*N+j] = sum`: index is a numeric operand, RHS may use `name` numerically.
        IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            !pi_mentions(object, name)
                && nus_num_operand(index, name, nums)
                && nus_expr(value, name, nums)
        }
        MemberAssign { object, value, .. } => {
            !pi_mentions(object, name) && nus_expr(value, name, nums)
        }
        // any other context where `name` could appear non-numerically -> require it absent.
        _ => !pi_mentions(e, name),
    }
}

/// A numeric-operand position: `name` may appear directly, or as a numeric sub-expr.
fn nus_num_operand(e: &Expr, name: &str, nums: &HashSet<String>) -> bool {
    if matches!(e, Expr::Ident { name: n, .. } if n.as_ref() == name) {
        return true;
    }
    nus_expr(e, name, nums)
}

/// A call argument: passing `name` BARE bails (callee param type unknown); a numeric sub-expr ok.
fn nus_arg(e: &Expr, name: &str, nums: &HashSet<String>) -> bool {
    if matches!(e, Expr::Ident { name: n, .. } if n.as_ref() == name) {
        return false;
    }
    nus_expr(e, name, nums)
}

/// Does `name` appear anywhere in `e`? Conservative: unhandled forms -> true (assume present).
pub(crate) fn pi_mentions(e: &Expr, name: &str) -> bool {
    use Expr::*;
    match e {
        Literal { .. } => false,
        Ident { name: n, .. } => n.as_ref() == name,
        Binary { left, right, .. } | NullishCoalesce { left, right, .. } => {
            pi_mentions(left, name) || pi_mentions(right, name)
        }
        Unary { operand, .. } | TypeOf { operand, .. } | Await { operand, .. } => {
            pi_mentions(operand, name)
        }
        Member { object, prop, .. } => {
            pi_mentions(object, name)
                || matches!(prop, tishlang_ast::MemberProp::Expr(p) if pi_mentions(p, name))
        }
        Index { object, index, .. } => pi_mentions(object, name) || pi_mentions(index, name),
        Call { callee, args, .. } | New { callee, args, .. } => {
            pi_mentions(callee, name)
                || args.iter().any(|a| match a {
                    tishlang_ast::CallArg::Expr(x) | tishlang_ast::CallArg::Spread(x) => {
                        pi_mentions(x, name)
                    }
                })
        }
        Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            pi_mentions(cond, name)
                || pi_mentions(then_branch, name)
                || pi_mentions(else_branch, name)
        }
        Assign { name: n, value, .. }
        | CompoundAssign { name: n, value, .. }
        | LogicalAssign { name: n, value, .. } => n.as_ref() == name || pi_mentions(value, name),
        PostfixInc { name: n, .. }
        | PostfixDec { name: n, .. }
        | PrefixInc { name: n, .. }
        | PrefixDec { name: n, .. } => n.as_ref() == name,
        Array { elements, .. } => elements.iter().any(|el| match el {
            tishlang_ast::ArrayElement::Expr(x) | tishlang_ast::ArrayElement::Spread(x) => {
                pi_mentions(x, name)
            }
        }),
        Object { props, .. } => props.iter().any(|p| match p {
            tishlang_ast::ObjectProp::KeyValue(_, v) => pi_mentions(v, name),
            tishlang_ast::ObjectProp::Spread(x) => pi_mentions(x, name),
        }),
        TemplateLiteral { exprs, .. } => exprs.iter().any(|x| pi_mentions(x, name)),
        MemberAssign { object, value, .. } => {
            pi_mentions(object, name) || pi_mentions(value, name)
        }
        IndexAssign {
            object,
            index,
            value,
            ..
        } => pi_mentions(object, name) || pi_mentions(index, name) || pi_mentions(value, name),
        // ArrowFunction (could capture), Jsx, native loads, etc. -> assume present (bail).
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Automatic struct inference (conservative, sound, opt-in)
// ---------------------------------------------------------------------------

/// One emitted `type` decl: alias name plus its field list.
type StructDecl = (String, Vec<(std::sync::Arc<str>, TypeAnnotation)>);

/// Registry of distinct inferred object shapes → synthetic alias name, so
/// identical shapes share one generated struct.
#[derive(Default)]
struct StructRegistry {
    /// canonical "k1:ty1;k2:ty2;…" → alias name
    by_shape: HashMap<String, String>,
    /// alias name → field list (for emitting the `type` decls)
    decls: Vec<StructDecl>,
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
    // Co-infer mutable `number[]` locals across the whole block (cross-referencing arrays) up front.
    // Co-infer mutable native arrays (number[] and boolean[]) across the block; an array of the
    // wrong element type simply fails its run and stays boxed.
    let native_num_arrays = block_native_arrays(&stmts, ctx, &number_ann());
    let native_bool_arrays = block_native_arrays(&stmts, ctx, &bool_ann());
    let n = stmts.len();
    let mut out: Vec<Statement> = Vec::with_capacity(n);
    for (i, stmt) in stmts.iter().enumerate() {
        // Candidate: `let xs = [ ...uniform native scalars... ]` with no annotation -> native `T[]`,
        // but only when every later use is read-only (`uses_are_array_safe`), so a `Vec<f64>`
        // assumption can't be violated by a later `push`/`xs[i] = …`.
        if let Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann: None,
            init: Some(Expr::Array { elements, .. }),
            span,
        } = stmt
        {
            // (a) Read-only typed array: uniform scalar literals, every later use read-only.
            if let Some(elem) = infer_array_elem(elements, ctx) {
                if uses_are_array_safe(name.as_ref(), &stmts[i + 1..]) {
                    let arr_ann = TypeAnnotation::Array(Box::new(elem));
                    ctx.define(name.as_ref(), arr_ann.clone());
                    out.push(Statement::VarDecl {
                        name: name.clone(),
                        name_span: *name_span,
                        mutable: *mutable,
                        type_ann: Some(arr_ann),
                        init: stmt_init_clone(stmt),
                        span: *span,
                    });
                    continue;
                }
            }
            // (b) Mutable native `number[]` / `boolean[]`: `let a = []` / `[lits]` driven by push +
            // index read/write (`fannkuch`/`queens`/`nsieve`, incl. cross-referencing siblings).
            // Sound via the block-level fixpoint above (every accepted array's elements are provably
            // of the chosen element type). `number[]` wins over `boolean[]` if somehow in both.
            let native_elem = if native_num_arrays.contains(name.as_ref()) {
                Some(number_ann())
            } else if native_bool_arrays.contains(name.as_ref()) {
                Some(bool_ann())
            } else {
                None
            };
            if let Some(elem) = native_elem {
                let arr_ann = TypeAnnotation::Array(Box::new(elem));
                ctx.define(name.as_ref(), arr_ann.clone());
                out.push(Statement::VarDecl {
                    name: name.clone(),
                    name_span: *name_span,
                    mutable: *mutable,
                    type_ann: Some(arr_ann),
                    init: stmt_init_clone(stmt),
                    span: *span,
                });
                continue;
            }
        }
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
        // Record a typed plain local (e.g. `let i = 0`) so a LATER object literal can type its
        // fields from it (`{ x: i }` → struct). The first inference pass may already have annotated
        // it (`let i: number`), so read the annotation OR infer from the init. Object-literal lets
        // defined their struct alias above and `continue`d; this runs for the rest. Without it,
        // `{ x: i }` can't resolve `i`'s type and the object stays a boxed `PropMap` (object_sum gap).
        if let Statement::VarDecl {
            name,
            type_ann,
            init,
            ..
        } = stmt
        {
            let t = type_ann
                .clone()
                .or_else(|| init.as_ref().and_then(|e| infer_expr_type(e, ctx)));
            if let Some(t) = t {
                ctx.define(name.as_ref(), t);
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
        } => {
            // Define the (annotated or M4-inferred) scalar params in a body scope so the body's
            // local-type + mutable-array inference can use them (`let r = n` ⇒ `r: number`).
            ctx.push_scope();
            for p in params {
                if let FunParam::Simple(tp) = p {
                    if let Some(ann) = &tp.type_ann {
                        ctx.define(tp.name.as_ref(), ann.clone());
                    }
                }
            }
            let new_body = Box::new(si_recurse(body, reg, ctx));
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

/// Define every provably-numeric block-local into `hyp` (flat across nested scopes): annotated
/// `: number`, a number-literal init, or any init `infer_expr_type` proves `number` *under the
/// current hypothesis* — so `let temp = perm[i]` becomes `number` once `perm` is hypothesized
/// `number[]`. Run a few times to resolve derived chains (`let b = temp + 1`).
fn seed_numeric_locals(s: &Statement, hyp: &mut InferCtx) {
    use Statement::*;
    match s {
        VarDecl { name, type_ann, init, .. } => {
            let numeric = type_ann.as_ref().is_some_and(is_number)
                || init
                    .as_ref()
                    .and_then(|e| infer_expr_type(e, hyp))
                    .as_ref()
                    .is_some_and(is_number);
            if numeric {
                hyp.define(name, number_ann());
            }
        }
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().for_each(|x| seed_numeric_locals(x, hyp))
        }
        If { then_branch, else_branch, .. } => {
            seed_numeric_locals(then_branch, hyp);
            if let Some(e) = else_branch {
                seed_numeric_locals(e, hyp);
            }
        }
        For { init, body, .. } => {
            if let Some(i) = init {
                seed_numeric_locals(i, hyp);
            }
            seed_numeric_locals(body, hyp);
        }
        While { body, .. } | DoWhile { body, .. } | ForOf { body, .. } => {
            seed_numeric_locals(body, hyp)
        }
        _ => {}
    }
}

/// Block-level co-inference of mutable native arrays of element type `elem` (`number[]` or
/// `boolean[]`) — handles cross-referencing arrays (`perm[i] = perm1[i]`) that a per-array pass can't.
/// Collects every top-level `let X = []` / `[elem literals]` candidate, then runs a monotone
/// **fixpoint**: hypothesize ALL candidates are `elem[]`, verify each (a candidate fails if any
/// value written/pushed isn't provably `elem` under the hypothesis, or it escapes), drop the
/// failures, repeat until stable. The stable set is self-consistent, so it's sound. Run once per
/// element type; an array of the wrong type simply fails this run (its pushes aren't `elem`).
fn block_native_arrays(
    stmts: &[Statement],
    outer_ctx: &InferCtx,
    elem: &TypeAnnotation,
) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut cands: Vec<(usize, String)> = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        if let Statement::VarDecl {
            name,
            type_ann: None,
            init: Some(Expr::Array { elements, .. }),
            ..
        } = s
        {
            // Empty `[]` is a candidate for either element type; literal elements must match `elem`.
            let lit_ok = elements.is_empty()
                || matches!(infer_array_elem(elements, outer_ctx), Some(t) if &t == elem);
            if lit_ok {
                cands.push((i, name.to_string()));
            }
        }
    }
    if cands.is_empty() {
        return HashSet::new();
    }
    let mut accepted: HashSet<String> = cands.iter().map(|c| c.1.clone()).collect();
    loop {
        let mut hyp = outer_ctx.clone();
        for n in &accepted {
            hyp.define(n, TypeAnnotation::Array(Box::new(elem.clone())));
        }
        // Seed numeric block-locals under the array hypothesis so values like `temp` in
        // `let temp = perm[i]; perm[k-i] = temp` are known (`perm[i]` is `elem` because `perm` is
        // hypothesized `elem[]`). A few passes resolve derived chains (`let b = temp + 1`).
        for _ in 0..4 {
            for s in stmts {
                seed_numeric_locals(s, &mut hyp);
            }
        }
        let mut removed = false;
        for (idx, name) in &cands {
            if accepted.contains(name)
                && !stmts[idx + 1..].iter().all(|s| mut_arr_stmt_ok(s, name, elem, &hyp))
            {
                accepted.remove(name);
                removed = true;
            }
        }
        if !removed {
            break;
        }
    }
    accepted
}

/// A value written into / pushed onto `name` must have the array's element type `elem`: a `name[_]`
/// self-read (already `elem` by hypothesis) or any expression `infer_expr_type` proves is `elem`.
fn mut_arr_value_ok(v: &Expr, name: &str, elem: &TypeAnnotation, hyp: &InferCtx) -> bool {
    if let Expr::Index { object, .. } = v {
        if matches!(object.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == name) {
            return true;
        }
    }
    infer_expr_type(v, hyp).as_ref() == Some(elem)
}

fn mut_arr_stmt_ok(s: &Statement, name: &str, elem: &TypeAnnotation, hyp: &InferCtx) -> bool {
    use Statement::*;
    match s {
        VarDecl { name: n, init, .. } => {
            n.as_ref() != name && init.as_ref().is_none_or(|e| mut_arr_expr_ok(e, name, elem, hyp))
        }
        ExprStmt { expr, .. } => mut_arr_expr_ok(expr, name, elem, hyp),
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().all(|s| mut_arr_stmt_ok(s, name, elem, hyp))
        }
        If { cond, then_branch, else_branch, .. } => {
            mut_arr_expr_ok(cond, name, elem, hyp)
                && mut_arr_stmt_ok(then_branch, name, elem, hyp)
                && else_branch.as_ref().is_none_or(|e| mut_arr_stmt_ok(e, name, elem, hyp))
        }
        While { cond, body, .. } => {
            mut_arr_expr_ok(cond, name, elem, hyp) && mut_arr_stmt_ok(body, name, elem, hyp)
        }
        DoWhile { body, cond, .. } => {
            mut_arr_stmt_ok(body, name, elem, hyp) && mut_arr_expr_ok(cond, name, elem, hyp)
        }
        For { init, cond, update, body, .. } => {
            init.as_ref().is_none_or(|i| mut_arr_stmt_ok(i, name, elem, hyp))
                && cond.as_ref().is_none_or(|c| mut_arr_expr_ok(c, name, elem, hyp))
                && update.as_ref().is_none_or(|u| mut_arr_expr_ok(u, name, elem, hyp))
                && mut_arr_stmt_ok(body, name, elem, hyp)
        }
        ForOf { name: n, iterable, body, .. } => {
            if n.as_ref() == name {
                return false;
            }
            let iter_ok = matches!(iterable, Expr::Ident { name: it, .. } if it.as_ref() == name)
                || mut_arr_expr_ok(iterable, name, elem, hyp);
            iter_ok && mut_arr_stmt_ok(body, name, elem, hyp)
        }
        Return { value, .. } => value.as_ref().is_none_or(|e| mut_arr_expr_ok(e, name, elem, hyp)),
        Throw { value, .. } => mut_arr_expr_ok(value, name, elem, hyp),
        Break { .. } | Continue { .. } | TypeAlias { .. } => true,
        _ => false, // switch / try / nested fn / etc: bail
    }
}

fn mut_arr_expr_ok(e: &Expr, name: &str, elem: &TypeAnnotation, hyp: &InferCtx) -> bool {
    use Expr::*;
    let is_name = |x: &Expr| matches!(x, Expr::Ident { name: n, .. } if n.as_ref() == name);
    match e {
        Literal { .. } => true,
        Ident { name: n, .. } => n.as_ref() != name, // bare escape is unsafe
        Index { object, index, .. } => {
            (is_name(object) || mut_arr_expr_ok(object, name, elem, hyp))
                && mut_arr_expr_ok(index, name, elem, hyp)
        }
        // `name[i] = v`: OK iff `v` has type `elem`; else recurse (writing to a DIFFERENT array).
        IndexAssign { object, index, value, .. } => {
            if is_name(object) {
                mut_arr_expr_ok(index, name, elem, hyp)
                    && mut_arr_value_ok(value, name, elem, hyp)
                    && mut_arr_expr_ok(value, name, elem, hyp)
            } else {
                mut_arr_expr_ok(object, name, elem, hyp)
                    && mut_arr_expr_ok(index, name, elem, hyp)
                    && mut_arr_expr_ok(value, name, elem, hyp)
            }
        }
        // `name.push(v…)`: OK iff each `v` has type `elem`. Any other method on `name`: bail.
        Call { callee, args, .. } => {
            if let Member { object, prop: tishlang_ast::MemberProp::Name { name: m, .. }, .. } =
                callee.as_ref()
            {
                if is_name(object) {
                    return m.as_ref() == "push"
                        && args.iter().all(|a| match a {
                            tishlang_ast::CallArg::Expr(v) => {
                                mut_arr_value_ok(v, name, elem, hyp) && mut_arr_expr_ok(v, name, elem, hyp)
                            }
                            tishlang_ast::CallArg::Spread(_) => false,
                        });
                }
            }
            mut_arr_expr_ok(callee, name, elem, hyp)
                && args.iter().all(|a| match a {
                    tishlang_ast::CallArg::Expr(v) => mut_arr_expr_ok(v, name, elem, hyp),
                    tishlang_ast::CallArg::Spread(_) => false,
                })
        }
        Member { object, prop, .. } => {
            if is_name(object) {
                matches!(prop, tishlang_ast::MemberProp::Name { name: p, .. } if p.as_ref() == "length")
            } else {
                mut_arr_expr_ok(object, name, elem, hyp)
            }
        }
        Binary { left, right, .. } => {
            mut_arr_expr_ok(left, name, elem, hyp) && mut_arr_expr_ok(right, name, elem, hyp)
        }
        Unary { operand, .. } => mut_arr_expr_ok(operand, name, elem, hyp),
        Conditional { cond, then_branch, else_branch, .. } => {
            mut_arr_expr_ok(cond, name, elem, hyp)
                && mut_arr_expr_ok(then_branch, name, elem, hyp)
                && mut_arr_expr_ok(else_branch, name, elem, hyp)
        }
        Assign { name: an, value, .. }
        | CompoundAssign { name: an, value, .. }
        | LogicalAssign { name: an, value, .. } => {
            an.as_ref() != name && mut_arr_expr_ok(value, name, elem, hyp)
        }
        PostfixInc { name: n, .. }
        | PostfixDec { name: n, .. }
        | PrefixInc { name: n, .. }
        | PrefixDec { name: n, .. } => n.as_ref() != name,
        MemberAssign { object, value, .. } => {
            mut_arr_expr_ok(object, name, elem, hyp) && mut_arr_expr_ok(value, name, elem, hyp)
        }
        // Anything else: `name` must be absent (would alias the Vec into a boxed context).
        _ => !pi_mentions(e, name),
    }
}

fn uses_are_array_safe(name: &str, tail: &[Statement]) -> bool {
    tail.iter().all(|s| arr_stmt_safe(s, name))
}

fn arr_opt_expr_safe(e: &Option<Expr>, name: &str) -> bool {
    e.as_ref().map(|e| arr_expr_safe(e, name)).unwrap_or(true)
}

fn arr_stmt_safe(s: &Statement, name: &str) -> bool {
    use Statement::*;
    match s {
        VarDecl { name: n, init, .. } => n.as_ref() != name && arr_opt_expr_safe(init, name),
        ExprStmt { expr, .. } => arr_expr_safe(expr, name),
        Block { statements, .. } => statements.iter().all(|s| arr_stmt_safe(s, name)),
        If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            arr_expr_safe(cond, name)
                && arr_stmt_safe(then_branch, name)
                && else_branch.as_ref().map(|e| arr_stmt_safe(e, name)).unwrap_or(true)
        }
        While { cond, body, .. } => arr_expr_safe(cond, name) && arr_stmt_safe(body, name),
        DoWhile { body, cond, .. } => arr_stmt_safe(body, name) && arr_expr_safe(cond, name),
        For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().map(|i| arr_stmt_safe(i, name)).unwrap_or(true)
                && cond.as_ref().map(|c| arr_expr_safe(c, name)).unwrap_or(true)
                && update.as_ref().map(|u| arr_expr_safe(u, name)).unwrap_or(true)
                && arr_stmt_safe(body, name)
        }
        // `for (_ of name)` is the key read-only use; rebinding `name` bails.
        ForOf {
            name: n,
            iterable,
            body,
            ..
        } => {
            if n.as_ref() == name {
                return false;
            }
            let iter_ok = matches!(iterable, Expr::Ident { name: it, .. } if it.as_ref() == name)
                || arr_expr_safe(iterable, name);
            iter_ok && arr_stmt_safe(body, name)
        }
        Return { value, .. } => arr_opt_expr_safe(value, name),
        Throw { value, .. } => arr_expr_safe(value, name),
        Break { .. } | Continue { .. } | TypeAlias { .. } => true,
        // Nested fn could capture+mutate; switch/try and anything else: be safe, bail.
        _ => false,
    }
}

fn arr_expr_safe(e: &Expr, name: &str) -> bool {
    use Expr::*;
    match e {
        Literal { .. } => true,
        Ident { name: n, .. } => n.as_ref() != name, // bare escape is unsafe
        // `name[i]` lowers to a native `Vec` index that PANICS out-of-bounds, whereas the boxed
        // array yields `undefined` — so an index read of `name` is unsound for inference. (Only
        // `for (_ of name)` and `name.length` are safe reads.) Indexing a DIFFERENT array is fine.
        Index { object, index, .. } => {
            !matches!(object.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == name)
                && arr_expr_safe(object, name)
                && arr_expr_safe(index, name)
        }
        // `name.length` READ is safe; any other `name.<prop>` (incl. a method receiver) bails.
        Member {
            object,
            prop,
            optional,
            ..
        } => {
            if let Ident { name: n, .. } = object.as_ref() {
                if n.as_ref() == name {
                    return !optional
                        && matches!(prop, tishlang_ast::MemberProp::Name { name: k, .. } if k.as_ref() == "length");
                }
            }
            arr_expr_safe(object, name)
                && match prop {
                    tishlang_ast::MemberProp::Expr(p) => arr_expr_safe(p, name),
                    tishlang_ast::MemberProp::Name { .. } => true,
                }
        }
        Binary { left, right, .. } | NullishCoalesce { left, right, .. } => {
            arr_expr_safe(left, name) && arr_expr_safe(right, name)
        }
        Unary { operand, .. } | TypeOf { operand, .. } | Await { operand, .. } => {
            arr_expr_safe(operand, name)
        }
        Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            arr_expr_safe(cond, name)
                && arr_expr_safe(then_branch, name)
                && arr_expr_safe(else_branch, name)
        }
        Call { callee, args, .. } | New { callee, args, .. } => {
            arr_expr_safe(callee, name)
                && args.iter().all(|a| match a {
                    tishlang_ast::CallArg::Expr(x) | tishlang_ast::CallArg::Spread(x) => {
                        arr_expr_safe(x, name)
                    }
                })
        }
        Assign { name: an, value, .. }
        | CompoundAssign { name: an, value, .. }
        | LogicalAssign { name: an, value, .. } => {
            an.as_ref() != name && arr_expr_safe(value, name) // reassigning `name` bails
        }
        // Mutating `name` via index/member assignment bails; otherwise recurse.
        IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            !matches!(object.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == name)
                && arr_expr_safe(object, name)
                && arr_expr_safe(index, name)
                && arr_expr_safe(value, name)
        }
        MemberAssign { object, value, .. } => {
            !matches!(object.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == name)
                && arr_expr_safe(object, name)
                && arr_expr_safe(value, name)
        }
        PostfixInc { name: n, .. }
        | PostfixDec { name: n, .. }
        | PrefixInc { name: n, .. }
        | PrefixDec { name: n, .. } => n.as_ref() != name,
        Array { elements, .. } => elements.iter().all(|el| match el {
            tishlang_ast::ArrayElement::Expr(x) | tishlang_ast::ArrayElement::Spread(x) => {
                arr_expr_safe(x, name)
            }
        }),
        Object { props, .. } => props.iter().all(|p| match p {
            tishlang_ast::ObjectProp::KeyValue(_, v) => arr_expr_safe(v, name),
            tishlang_ast::ObjectProp::Spread(v) => arr_expr_safe(v, name),
        }),
        // Anything else that mentions `name`: be safe, bail.
        _ => !pi_mentions(e, name),
    }
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
        Delete { target, .. } => expr_name_safe(target, name, keys),
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

#[cfg(test)]
mod param_infer_tests {
    use super::*;
    use tishlang_parser::parse;

    /// Run base inference, then M4 param inference, and return the inferred annotation name (if
    /// any) for parameter `param` of `fn <fn_name>`.
    fn inferred_param(src: &str, fn_name: &str, param: &str) -> Option<String> {
        let parsed = parse(src).unwrap();
        let base = Program {
            statements: infer_statements(&parsed.statements, &mut InferCtx::new()),
        };
        let prog = param_infer_program(base);
        for s in &prog.statements {
            if let Statement::FunDecl { name, params, .. } = s {
                if name.as_ref() == fn_name {
                    for p in params {
                        if let FunParam::Simple(tp) = p {
                            if tp.name.as_ref() == param {
                                return tp.type_ann.as_ref().map(|a| match a {
                                    TypeAnnotation::Simple(s) => s.to_string(),
                                    _ => "<complex>".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
        None
    }

    #[test]
    fn infers_loop_bound_param_via_numeric_local() {
        // `n` is the bare operand of `i < n`; `i` (`let i = 0`, base-inferred numeric) makes the
        // OTHER operand provably numeric, so `n` is inferred `number` (the numeric-locals fix).
        let src = "fn countUp(n) { let total = 0; for (let i = 0; i < n; i = i + 1) { total = total + i } return total }";
        assert_eq!(inferred_param(src, "countUp", "n").as_deref(), Some("number"));
    }

    #[test]
    fn does_not_infer_string_concat_param() {
        // `x` is the bare operand of `+` against a string literal — NOT provably numeric, so `x`
        // must stay dynamic (else `label("hi")` would mistype to f64 and panic at runtime).
        let src = "fn label(x) { return \"v=\" + x }";
        assert_eq!(inferred_param(src, "label", "x"), None);
    }

    #[test]
    fn does_not_treat_other_param_as_numeric_local() {
        // `a < b`: neither operand is a known numeric *local* (both are params), so neither is
        // provable and neither param is inferred — the relaxation is locals-only.
        let src = "fn cmp(a, b) { if (a < b) { return 1 } return 0 }";
        assert_eq!(inferred_param(src, "cmp", "a"), None);
        assert_eq!(inferred_param(src, "cmp", "b"), None);
    }
}
