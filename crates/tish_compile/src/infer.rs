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
    /// #175: fns de-virtualized to native-vec free fns → per-param "is an array param" flags. Lets
    /// the mutable-array co-inference treat `f(arr)` as a native use (the callee takes `&/&mut Vec`),
    /// not a boxing escape, so the caller's array stays an unboxed `Vec`.
    native_vec_array_params: HashMap<String, Vec<bool>>,
    /// #320: fns PROVEN to always return a `number` (every return is numeric and the body can't fall
    /// through to an implicit `undefined`). Lets `infer_expr_type(f(...))` be `number`, so
    /// `a.push(f(...))` infers `a: number[]` (e.g. k_nucleotide's `seq.push(nextBase())`).
    number_returning_fns: std::collections::HashSet<String>,
}

impl InferCtx {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            native_vec_array_params: HashMap::new(),
            number_returning_fns: std::collections::HashSet::new(),
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
    matches!(ann, TypeAnnotation::Simple(s, _) if s.as_ref() == "number")
}

fn is_string(ann: &TypeAnnotation) -> bool {
    matches!(ann, TypeAnnotation::Simple(s, _) if s.as_ref() == "string")
}

fn is_bool(ann: &TypeAnnotation) -> bool {
    matches!(ann, TypeAnnotation::Simple(s, _) if s.as_ref() == "boolean")
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
    TypeAnnotation::Simple("number".into(), tishlang_ast::Span::default())
}

fn string_ann() -> TypeAnnotation {
    TypeAnnotation::Simple("string".into(), tishlang_ast::Span::default())
}

fn bool_ann() -> TypeAnnotation {
    TypeAnnotation::Simple("boolean".into(), tishlang_ast::Span::default())
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
        // #320: a call to a fn PROVEN to always return a number is itself a number — so
        // `a.push(f(...))` can infer `a: number[]` (e.g. `seq.push(nextBase())`).
        Expr::Call { callee, .. } => match callee.as_ref() {
            Expr::Ident { name, .. } if ctx.number_returning_fns.contains(name.as_ref()) => {
                Some(number_ann())
            }
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
    let p = if std::env::var("TISH_STRUCT_INFER").map(|v| v != "0").unwrap_or(false) {
        struct_infer_program(p)
    } else {
        p
    };
    // S-0..S-C aggregate (interprocedural monomorphic struct) inference — issue #177, opt-in via
    // TISH_AGGREGATE_INFER, OFF by default. Front-end of the nbody unboxing lever: it
    //   S-0: types params used ONLY as object-literal field values (`body(x,…)` → `: number`),
    //   S-A: registers the return-shape struct alias for an all-f64 object-literal-returning fn,
    //   S-B: propagates a call's return type to `let p = body(…)` / `let bs = makeBodies()`,
    //   S-C: types `[ident,…]` array literals from those struct-typed locals.
    // The full lever also needs S-D (write-permitting param-shape) + S-E/S-F (the typed-fn ABI
    // tier in codegen) before the annotations can be *consumed* without a boxed-edge miscompile;
    // until those land this pass only emits the annotations the existing codegen backs SOUNDLY
    // (the S-0 scalar `: number` params, identical to the M4 mechanism) plus the inert struct
    // alias decls. See `aggregate_infer_program`.
    if std::env::var("TISH_AGGREGATE_INFER").map(|v| v != "0").unwrap_or(false) {
        aggregate_infer_program(p)
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
        // Fixpoint-closed so copy-chains of numeric literals (`let a = 0; let b = a`) propagate.
        let mut base_nums = HashSet::new();
        collect_numeric_locals_fixpoint(&body, &mut base_nums);
        let new_params = params
            .into_iter()
            .map(|p| match p {
                FunParam::Simple(mut tp) => {
                    // OPTIMISTIC PER-PARAM FIXPOINT (#172): assume the candidate param numeric, then
                    // propagate its copies — `let r = n` makes `r` numeric — to a fixpoint, so the
                    // chicken-and-egg `n` needs `r` (via `r === n`); `r` (`let r = n`) needs `n`
                    // closes. We then VERIFY every use of `n` is numeric-safe (`nus_stmt`) under this
                    // augmented set; a genuinely non-numeric use (string concat, object store, bare
                    // escape) still bails, so `fn label(x){return "v="+x}` keeps `x` dynamic.
                    //
                    // SOUNDNESS of the copy laundering: a copy `let r = n` only stays sound if `r`
                    // ITSELF is used numeric-safely everywhere — else `let r = n; r + "!"` would
                    // type `n` numeric yet do string concat (a divergence). So we also require every
                    // local the fixpoint added *because of this candidate* (in `nums` but not in
                    // `base_nums` — i.e. a copy-descendant of `n`) to pass `lns_stmt` (the
                    // local-numeric verifier). A poisoned copy-target bails the whole param.
                    // Monotone, locals/param-candidate-only.
                    if tp.type_ann.is_none()
                        && tp.default.is_none()
                    {
                        let mut nums = base_nums.clone();
                        nums.insert(tp.name.to_string());
                        collect_numeric_locals_fixpoint(&body, &mut nums);
                        let copy_descendants: Vec<&String> = nums
                            .iter()
                            .filter(|x| x.as_str() != tp.name.as_ref() && !base_nums.contains(*x))
                            .collect();
                        if nus_stmt(&body, tp.name.as_ref(), &nums)
                            && copy_descendants
                                .iter()
                                .all(|x| lns_stmt(&body, x.as_str(), &nums))
                        {
                            tp.type_ann = Some(TypeAnnotation::Simple(std::sync::Arc::from("number"), tishlang_ast::Span::default()));
                        }
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
///
/// MONOTONE FIXPOINT (#172): a single pass seeds number-LITERAL / annotated inits AND propagates
/// bare-ident copies — `let x = y` where `y` is ALREADY in `out` (a copy of an already-numeric
/// local/param). Run to a fixpoint via `collect_numeric_locals_fixpoint` so a chain
/// `let r = n; let s = r` closes. Soundness is identical to the literal seeding: a copy of a value
/// known numeric is itself numeric; over-inclusion only widens, bounded by M4's
/// caller-passes-a-number / NaN-coercion contract and the gauntlet/corpus differential oracles.
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
                )
                // FIXPOINT: a bare-ident copy of an already-numeric local/param — `let r = n` where
                // `n` is already known numeric. The copied value carries the source's numeric type.
                || matches!(
                    init,
                    Some(tishlang_ast::Expr::Ident { name: src, .. }) if out.contains(src.as_ref())
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

/// Run `collect_numeric_locals` to a monotone fixpoint over `body`, seeding `out` with any
/// pre-known-numeric names (e.g. the param-candidate assumed numeric). Each pass can only ADD
/// names (a copy of an already-numeric source), so it converges in at most one pass per copy-chain
/// link; the `out.len()` watermark detects quiescence. Soundness: every added name is a copy of a
/// value already proven/assumed numeric, the same basis as the literal seeding.
fn collect_numeric_locals_fixpoint(body: &Statement, out: &mut HashSet<String>) {
    loop {
        let before = out.len();
        collect_numeric_locals(body, out);
        if out.len() == before {
            break;
        }
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
        } => {
            // Writing the candidate to a name that shadows it bails (its type could change).
            // Otherwise a bare-ident COPY of the candidate — `let r = n` — is a numeric-safe use:
            // the param's value flows into a local that the per-param fixpoint already proved
            // numeric (`r ∈ nums`), so it lowers to `f64`. This is what gates the fannkuch cascade
            // (#172). Any other init form is checked by `nus_expr` as before (so `let s = n + ":"`
            // still bails via the overloaded-`+` rule, keeping string-concat params dynamic).
            vn.as_ref() != name
                && init.as_ref().is_none_or(|e| {
                    matches!(e, Expr::Ident { name: src, .. } if src.as_ref() == name)
                        || nus_expr(e, name, nums)
                })
        }
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

// ---------------------------------------------------------------------------
// LOCAL-numeric-safe (#172): verify a LOCAL `loc` that the per-param copy-fixpoint marked numeric
// (a copy-descendant of the candidate param, e.g. `r` in `let r = n`) is used consistently as a
// number throughout the body — so the param→local laundering can't mask a non-numeric use. This is
// the dual of `nus_*` (which is for params), differing in exactly the local-only safe forms:
//   * a bare read `loc` is a numeric VALUE (it IS a number) → safe in any value position;
//   * the DEFINING `let loc = <numeric/copy>` and self-reassign `loc = <numeric>` keep it numeric;
//   * the SAME bail set as params — `loc + <non-numeric>` (string concat), member/object store,
//     bare call-arg escape, index-OBJECT use — still bails, so `let r = n; r + "!"` poisons it.
// `nums` already contains every numeric local/param-candidate, so the overloaded-`+`/comparison
// "other side provably numeric" rule reuses `numeric_provable` unchanged.
// ---------------------------------------------------------------------------

/// Every use of the known-numeric local `loc` in `s` is consistent with `loc` being a number.
fn lns_stmt(s: &Statement, loc: &str, nums: &HashSet<String>) -> bool {
    use Statement::*;
    match s {
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().all(|x| lns_stmt(x, loc, nums))
        }
        Return { value, .. } => value.as_ref().is_none_or(|e| lns_num_operand(e, loc, nums)),
        If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            lns_expr(cond, loc, nums)
                && lns_stmt(then_branch, loc, nums)
                && else_branch.as_ref().is_none_or(|e| lns_stmt(e, loc, nums))
        }
        ExprStmt { expr, .. } => lns_expr(expr, loc, nums),
        While { cond, body, .. } => lns_expr(cond, loc, nums) && lns_stmt(body, loc, nums),
        For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_none_or(|x| lns_stmt(x, loc, nums))
                && cond.as_ref().is_none_or(|e| lns_expr(e, loc, nums))
                && update.as_ref().is_none_or(|e| lns_expr(e, loc, nums))
                && lns_stmt(body, loc, nums)
        }
        VarDecl {
            name: vn, init, ..
        } => {
            if vn.as_ref() == loc {
                // The DEFINING decl of `loc`: a numeric / copy-of-numeric init keeps it numeric.
                // (Re-`let` to a non-numeric value would poison it — require a provable init.)
                init.as_ref().is_none_or(|e| lns_def_init_ok(e, nums))
            } else {
                init.as_ref().is_none_or(|e| lns_expr(e, loc, nums))
            }
        }
        Break { .. } | Continue { .. } => true,
        _ => false,
    }
}

/// The init/RHS of a numeric local's defining decl or self-assign: must be PROVABLY numeric (a
/// number literal / arithmetic / Math / a bare numeric local incl. the param-candidate). This is
/// `numeric_provable` over `nums` — bare `loc` itself counts (it is numeric).
fn lns_def_init_ok(e: &Expr, nums: &HashSet<String>) -> bool {
    use Expr::*;
    match e {
        // Bare ident: a copy of a numeric local/param-candidate (`let r = n`, `r = i`).
        Ident { name: n, .. } => nums.contains(n.as_ref()),
        // `r = r - 1`, `r = r + 1` etc.: overloaded ops need at least one provably-numeric operand,
        // which `numeric_provable` already enforces for the non-`Add` family; for `+` we require
        // BOTH sides numeric-provable (so `r + "!"` is rejected — string concat poisons `r`).
        Binary {
            left, op, right, ..
        } => {
            use tishlang_ast::BinOp::*;
            match op {
                Sub | Mul | Div | Mod | Pow | BitAnd | BitOr | BitXor | Shl | Shr | UShr | Add => {
                    numeric_provable(left, nums) && numeric_provable(right, nums)
                }
                _ => false,
            }
        }
        _ => numeric_provable(e, nums),
    }
}

/// `e` with the known-numeric local `loc` used only where a number is valid.
fn lns_expr(e: &Expr, loc: &str, nums: &HashSet<String>) -> bool {
    use Expr::*;
    match e {
        Literal { .. } => true,
        // Bare read of `loc` (a number) — or of any other ident — is a numeric VALUE, always safe.
        Ident { .. } => true,
        Binary {
            left, op, right, ..
        } => {
            use tishlang_ast::BinOp::*;
            match op {
                Sub | Mul | Div | Mod | Pow | BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                    lns_num_operand(left, loc, nums) && lns_num_operand(right, loc, nums)
                }
                // OVERLOADED: if `loc` is a direct operand, the OTHER side must be provably numeric
                // — so `loc + "!"` (string concat) bails, poisoning the candidate.
                Add | Lt | Le | Gt | Ge | StrictEq | StrictNe => {
                    lns_overloaded(left, right, loc, nums) && lns_overloaded(right, left, loc, nums)
                }
                And | Or => lns_expr(left, loc, nums) && lns_expr(right, loc, nums),
                _ => !pi_mentions(left, loc) && !pi_mentions(right, loc),
            }
        }
        Unary { op, operand, .. } => {
            if matches!(
                op,
                tishlang_ast::UnaryOp::Neg | tishlang_ast::UnaryOp::Pos | tishlang_ast::UnaryOp::BitNot
            ) {
                lns_num_operand(operand, loc, nums)
            } else {
                !pi_mentions(operand, loc)
            }
        }
        // `a[loc]`: index is a numeric operand; `loc` must NOT be the array object (member-ish use).
        Index { object, index, .. } => {
            !pi_mentions(object, loc) && lns_num_operand(index, loc, nums)
        }
        Call { callee, args, .. } => {
            !pi_mentions(callee, loc)
                && args.iter().all(|a| match a {
                    tishlang_ast::CallArg::Expr(x) => lns_arg(x, loc, nums),
                    tishlang_ast::CallArg::Spread(_) => false,
                })
        }
        Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            lns_expr(cond, loc, nums)
                && lns_num_operand(then_branch, loc, nums)
                && lns_num_operand(else_branch, loc, nums)
        }
        // Self-assign `loc = <numeric>` keeps it numeric; assign to another var may read `loc`.
        Assign { name: an, value, .. }
        | CompoundAssign { name: an, value, .. }
        | LogicalAssign { name: an, value, .. } => {
            if an.as_ref() == loc {
                lns_def_init_ok(value, nums)
            } else {
                lns_expr(value, loc, nums)
            }
        }
        // `loc++` / `--loc`: numeric in/decrement keeps `loc` numeric.
        PostfixInc { .. } | PostfixDec { .. } | PrefixInc { .. } | PrefixDec { .. } => true,
        // `c[idx] = <val>`: index & val are numeric-operand positions (bare `loc` value is a number).
        IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            !pi_mentions(object, loc)
                && lns_num_operand(index, loc, nums)
                && lns_num_operand(value, loc, nums)
        }
        MemberAssign { object, value, .. } => {
            !pi_mentions(object, loc) && lns_expr(value, loc, nums)
        }
        _ => !pi_mentions(e, loc),
    }
}

/// One side of an overloaded binop for a numeric local: if it is bare `loc`, the OTHER side must be
/// provably numeric; otherwise recurse.
fn lns_overloaded(operand: &Expr, other: &Expr, loc: &str, nums: &HashSet<String>) -> bool {
    if matches!(operand, Expr::Ident { name: n, .. } if n.as_ref() == loc) {
        return numeric_provable(other, nums);
    }
    lns_expr(operand, loc, nums)
}

/// A numeric-operand position for a numeric local: bare `loc` is a number; else a numeric sub-expr.
fn lns_num_operand(e: &Expr, loc: &str, nums: &HashSet<String>) -> bool {
    if matches!(e, Expr::Ident { name: n, .. } if n.as_ref() == loc) {
        return true;
    }
    lns_expr(e, loc, nums)
}

/// A call argument for a numeric local: passing `loc` BARE bails (callee param type unknown).
fn lns_arg(e: &Expr, loc: &str, nums: &HashSet<String>) -> bool {
    if matches!(e, Expr::Ident { name: n, .. } if n.as_ref() == loc) {
        return false;
    }
    lns_expr(e, loc, nums)
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
            tishlang_ast::ObjectProp::KeyValue(_, v, _) => pi_mentions(v, name),
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
        TypeAnnotation::Simple(s, _) => s.to_string(),
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
            tishlang_ast::ObjectProp::KeyValue(k, v, _) => {
                let ty = infer_expr_type(v, ctx)?;
                // Only primitive field types in this conservative version.
                if !matches!(&ty, TypeAnnotation::Simple(s, _)
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

/// #320: the set of top-level fns PROVEN to ALWAYS return a `number` — every `return` carries a
/// numeric value and the body can't fall through to an implicit `undefined`. Lets `infer_expr_type`
/// type a call `f(...)` as `number`, so `seq.push(nextBase())` infers `seq: number[]` (native key
/// arithmetic over a `Vec<f64>` instead of a boxed `Value[]`). Conservative + sound: any fn we
/// can't prove is left out, and the Call arm then declines to type its calls. Small fixpoint so a
/// number-fn may call another already-accepted number-fn.
fn collect_number_returning_fns(
    stmts: &[Statement],
    base: &InferCtx,
) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut accepted: HashSet<String> = HashSet::new();
    loop {
        let mut changed = false;
        for s in stmts {
            if let Statement::FunDecl {
                async_: false,
                name,
                params,
                rest_param: None,
                body,
                ..
            } = s
            {
                if accepted.contains(name.as_ref()) {
                    continue;
                }
                // Fn-local ctx: numeric params (param-infer already annotated them), numeric locals,
                // and the number-fns accepted so far (so a numeric call inside resolves).
                let mut fctx = base.clone();
                fctx.number_returning_fns = accepted.clone();
                fctx.push_scope();
                for p in params {
                    if let FunParam::Simple(tp) = p {
                        if tp.type_ann.as_ref().is_some_and(is_number) {
                            fctx.define(&tp.name, number_ann());
                        }
                    }
                }
                // A few passes so chained numeric locals settle (`let a = 0; let b = a * 2`).
                for _ in 0..4 {
                    seed_numeric_locals(body, &mut fctx);
                }
                if fn_always_returns_number(body, &fctx) {
                    accepted.insert(name.to_string());
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    accepted
}

/// A fn always returns a number iff EVERY `return` in its body carries a value that infers to
/// `number` AND the body can't fall through to an implicit `undefined` (conservatively: its last
/// statement is an unconditional `return <value>`).
fn fn_always_returns_number(body: &Statement, ctx: &InferCtx) -> bool {
    let mut ok = true;
    check_returns_numeric(body, ctx, &mut ok);
    ok && body_ends_in_return(body)
}

/// Visit EVERY `return` reachable in THIS fn (descends all statement-nesting constructs but NOT
/// nested fn bodies, whose returns belong to the closure). Sets `ok = false` on a bare `return;` or
/// a `return <e>` whose value doesn't provably infer to `number`. Missing a construct here would be
/// unsound, so every statement-nesting variant is enumerated explicitly.
fn check_returns_numeric(s: &Statement, ctx: &InferCtx, ok: &mut bool) {
    use Statement::*;
    if !*ok {
        return;
    }
    match s {
        Return { value, .. } => match value {
            Some(e) => {
                if infer_expr_type(e, ctx).as_ref().map_or(true, |t| !is_number(t)) {
                    *ok = false;
                }
            }
            None => *ok = false,
        },
        FunDecl { .. } => {} // nested fn: its returns are not this fn's
        Block { statements, .. } | Multi { statements, .. } => {
            for c in statements {
                check_returns_numeric(c, ctx, ok);
            }
        }
        If { then_branch, else_branch, .. } => {
            check_returns_numeric(then_branch, ctx, ok);
            if let Some(e) = else_branch {
                check_returns_numeric(e, ctx, ok);
            }
        }
        While { body, .. } | DoWhile { body, .. } | ForOf { body, .. } => {
            check_returns_numeric(body, ctx, ok);
        }
        For { init, body, .. } => {
            if let Some(i) = init {
                check_returns_numeric(i, ctx, ok);
            }
            check_returns_numeric(body, ctx, ok);
        }
        Switch { cases, default_body, .. } => {
            for (_, body) in cases {
                for c in body {
                    check_returns_numeric(c, ctx, ok);
                }
            }
            if let Some(d) = default_body {
                for c in d {
                    check_returns_numeric(c, ctx, ok);
                }
            }
        }
        Try { body, catch_body, finally_body, .. } => {
            check_returns_numeric(body, ctx, ok);
            if let Some(c) = catch_body {
                check_returns_numeric(c, ctx, ok);
            }
            if let Some(f) = finally_body {
                check_returns_numeric(f, ctx, ok);
            }
        }
        _ => {}
    }
}

/// Conservative "the body always reaches a return": the last statement is an unconditional
/// `return <value>` (or a nested block that itself ends in one). Anything else (trailing `if`,
/// loop, fall-off) is treated as a possible implicit-`undefined` exit and rejects the fn.
fn body_ends_in_return(s: &Statement) -> bool {
    match s {
        Statement::Return { value: Some(_), .. } => true,
        Statement::Block { statements, .. } => statements.last().is_some_and(body_ends_in_return),
        _ => false,
    }
}

fn struct_infer_program(program: Program) -> Program {
    let mut reg = StructRegistry::default();
    let mut ctx = InferCtx::new();
    // #320: fns proven to always return a number, so `infer_expr_type(f(...))` is `number` and a
    // `seq.push(nextBase())` keeps `seq` a native `number[]`. Computed off the param-inferred
    // program so numeric params are already typed.
    ctx.number_returning_fns = collect_number_returning_fns(&program.statements, &ctx);
    // #175: tell the mutable-array co-inference which fns take native `&/&mut Vec` array params, so
    // forwarding an array into one isn't treated as a boxing escape (lets the caller's array stay
    // unboxed). Same AST-only detection codegen uses, so the two never disagree.
    if std::env::var("TISH_NATIVE_FN").map(|v| v != "0").unwrap_or(false) {
        for (fname, sig) in crate::codegen::Codegen::detect_native_vec_fns(&program) {
            let flags: Vec<bool> = sig
                .params
                .iter()
                .map(|(_, k)| matches!(k, crate::codegen::VecParamKind::Array { .. }))
                .collect();
            ctx.native_vec_array_params.insert(fname, flags);
        }
    }
    // #320: mark a read-only `number[]`-param fn's array args as non-escapes so the caller's array
    // stays `number[]` (e.g. `kNucleotide(seq, k)`). This uses the READ-ONLY criterion only — sound
    // regardless of the arg type, since passing a native `Vec` to a boxed closure copies it (a
    // read-only callee can't mutate the caller's array through the copy). Codegen separately proves,
    // via the call sites, which of these it may actually UNBOX (`native_arr_param_fns`); a fn it
    // can't prove simply stays a fully boxed closure. Runs pre-`si_block`, so it must not rely on
    // `number[]` stamps — `arr_param_readonly_fns` only inspects each fn's own body.
    if std::env::var("TISH_NATIVE_ARR_PARAM").map(|v| v != "0").unwrap_or(false) {
        for (fname, flags) in crate::codegen::Codegen::arr_param_readonly_fns(&program) {
            ctx.native_vec_array_params.insert(fname, flags);
        }
    }
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
                    ctx.define(name.as_ref(), TypeAnnotation::Simple(alias.as_str().into(), tishlang_ast::Span::default()));
                    out.push(Statement::VarDecl {
                        name: name.clone(),
                        name_span: *name_span,
                        mutable: *mutable,
                        type_ann: Some(TypeAnnotation::Simple(alias.as_str().into(), tishlang_ast::Span::default())),
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
            name_span,
            mutable,
            type_ann,
            init,
            span,
        } = stmt
        {
            let inferred = type_ann
                .clone()
                .or_else(|| init.as_ref().and_then(|e| infer_expr_type(e, ctx)));
            if let Some(t) = &inferred {
                ctx.define(name.as_ref(), t.clone());
            }
            // #170: the base inference pass ran before array types were known, so an unannotated
            // local whose init only types now that `perm: number[]` is known (`let k = perm[0]`,
            // `let temp = perm[i]`) was left `type_ann: None` — and codegen reads the NODE, not
            // `ctx`, so it stayed a boxed `Value` despite being provably `f64`. Persist a proven
            // `number` type back onto the node here (the second, type-aware pass). The codegen
            // demote-gate (`collect_demoted_numeric_locals`) re-boxes any such local whose later
            // reassignment can escape `number`, so writing the annotation can never miscompile.
            if type_ann.is_none() {
                if let Some(t @ TypeAnnotation::Simple(s, _)) = &inferred {
                    if s.as_ref() == "number" {
                        out.push(Statement::VarDecl {
                            name: name.clone(),
                            name_span: *name_span,
                            mutable: *mutable,
                            type_ann: Some(t.clone()),
                            init: stmt_init_clone(stmt),
                            span: *span,
                        });
                        continue;
                    }
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
        FunDecl { body, .. } => mut_arr_stmt_ok(body, name, elem, hyp),
        _ => false, // switch / try / etc: bail
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
            // #175: forwarding `name` into a native-vec fn's ARRAY param is a native use, not a
            // boxing escape — the callee takes it by `&/&mut Vec<elem>`, so the array stays unboxed.
            if let Ident { name: fname, .. } = callee.as_ref() {
                if let Some(is_arr) = hyp.native_vec_array_params.get(fname.as_ref()) {
                    if args.len() == is_arr.len() {
                        return args.iter().enumerate().all(|(i, a)| match a {
                            tishlang_ast::CallArg::Expr(v) => {
                                (is_name(v) && is_arr[i]) || mut_arr_expr_ok(v, name, elem, hyp)
                            }
                            tishlang_ast::CallArg::Spread(_) => false,
                        });
                    }
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
        FunDecl { body, .. } => arr_stmt_safe(body, name),
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
            tishlang_ast::ObjectProp::KeyValue(_, v, _) => arr_expr_safe(v, name),
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
            tishlang_ast::ObjectProp::KeyValue(_, v, _) => expr_name_safe(v, name, keys),
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

// ===========================================================================
// S-0..S-C: aggregate (interprocedural monomorphic struct) inference — issue #177
// ===========================================================================
//
// This is the front-end of the nbody-unboxing lever. It runs ONLY under
// `TISH_AGGREGATE_INFER` (OFF by default). The four sub-passes are pure analysis
// over the (already base/param/struct-inferred) `Program`:
//
//   S-0  param-numeric-as-field-value: a param used ONLY as an object-literal
//        field value (and otherwise numeric-safely) is `: number`. This MUST run
//        before S-A so `infer_object_shape(body)` can see the param types.
//   S-A  return-shape: a fn whose EVERY `return` is the same all-f64 object
//        literal gets a registered struct alias as its inferred return shape.
//   S-B  call-return propagation: `let p = body(…)` → struct; a fn that returns
//        an array of struct-typed idents → `Array(struct)`, so `let bs =
//        makeBodies()` is `Array(struct)`.
//   S-C  array-of-ident element typing: `[a, b, c]` where each ident is the same
//        struct alias → `Array(struct)`.
//
// SOUNDNESS / WHAT IS ACTUALLY WRITTEN BACK
// -----------------------------------------
// The struct types S-A/S-B/S-C compute cannot yet be *consumed* by codegen
// without the S-D write-permitting param predicate and the S-E/S-F typed-fn ABI
// tier (a de-virtualized `fn advance(bodies: &VmRef<Vec<TishStruct_Body>>, …)`).
// Until those land, writing a `Named`/`Array(Named)` annotation onto a fn param
// or a call-initialised local would MISCOMPILE: `collect_annotated_types`
// (codegen.rs) records the param/local as a native struct while the actual
// binding is still a boxed `Value` (the call returns boxed `Value` — there is no
// FnSigTable / struct-returning emission), so a `p.x` field read or `bodies[i].vx
// = …` write would be lowered against a boxed value. The boxed-edge `===`/escape
// hazards in the design's candidacy predicate are the same class of problem.
//
// Therefore `aggregate_infer_program` writes back ONLY the annotations the
// EXISTING codegen backs soundly:
//   * S-0's scalar `: number` params (identical to the M4 param-infer mechanism,
//     already consumed soundly), and
//   * the inert struct `type` alias decls (unreferenced aliases are dropped by
//     codegen — they change nothing on their own).
// The S-A/S-B/S-C struct *shapes* are computed and exposed via
// `analyze_aggregate` for unit tests and for the future S-D..S-F consumers, but
// are NOT yet stamped onto params / call-locals. This keeps the ON path free of
// any checksum divergence while the inference logic is validated independently.

/// The result of the aggregate analysis: the registered struct shapes plus, per
/// function, the inferred return shape and the set of params S-0 typed numeric.
#[derive(Default, Debug, Clone)]
pub struct AggregateAnalysis {
    /// alias name → ordered field list (all `number`), one per distinct shape.
    pub struct_decls: Vec<StructDecl>,
    /// fn name → its return shape: `Struct(alias)` or `ArrayOfStruct(alias)`.
    pub fn_return: HashMap<String, AggReturn>,
    /// fn name → set of param names S-0 proved numeric (object-field-value-only).
    pub fn_numeric_params: HashMap<String, HashSet<String>>,
    /// top-level `let` name → inferred aggregate type (S-B/S-C), for tests.
    pub local_agg: HashMap<String, AggReturn>,
    /// S-D: the whole-program unboxing verdict. `Some(alias)` ⇒ the struct alias
    /// `alias` is fully unboxable: its factory, array-factory, the top-level
    /// `Array(alias)` local(s), and every fn taking that array by param pass the
    /// all-or-nothing candidacy predicate, so codegen may emit the typed free-fn
    /// tier. `None` ⇒ bail the whole group to boxed (S-0 scalars still stamp).
    pub unbox_alias: Option<String>,
    /// S-D: fn name → (param-index, param-name) of the `Array(alias)` param, for
    /// the fns in the unbox group. Only populated when `unbox_alias` is `Some`.
    pub array_param_fns: HashMap<String, (usize, String)>,
    /// S-D: top-level `let` names whose inferred type is `Array(unbox_alias)`.
    pub array_locals: Vec<String>,
}

/// An inferred aggregate (struct-ish) type produced by the S-A..S-C passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggReturn {
    /// A monomorphic all-f64 struct shape, identified by its registered alias.
    Struct(String),
    /// An array whose elements are all the same `Struct(alias)`.
    ArrayOfStruct(String),
}

/// S-0 predicate: every use of `name` in `s` is EITHER a numeric-operand use
/// (per the existing `nus_*` rules) OR an object-literal field value `{ k: name }`.
/// Any other use — write to `name`, escape as a bare call-arg, member/index of
/// `name`, `===` on `name`, etc. — bails (returns false). `nums` carries the
/// numeric-local set so overloaded `+`/comparisons resolve as in `nus_*`.
fn pus_stmt(s: &Statement, name: &str, nums: &HashSet<String>) -> bool {
    use Statement::*;
    match s {
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().all(|x| pus_stmt(x, name, nums))
        }
        Return { value, .. } => value.as_ref().is_none_or(|e| pus_value(e, name, nums)),
        ExprStmt { expr, .. } => pus_value(expr, name, nums),
        VarDecl { name: vn, init, .. } => {
            vn.as_ref() != name && init.as_ref().is_none_or(|e| pus_value(e, name, nums))
        }
        If { cond, then_branch, else_branch, .. } => {
            pus_value(cond, name, nums)
                && pus_stmt(then_branch, name, nums)
                && else_branch.as_ref().is_none_or(|e| pus_stmt(e, name, nums))
        }
        While { cond, body, .. } => pus_value(cond, name, nums) && pus_stmt(body, name, nums),
        For { init, cond, update, body, .. } => {
            init.as_ref().is_none_or(|x| pus_stmt(x, name, nums))
                && cond.as_ref().is_none_or(|e| pus_value(e, name, nums))
                && update.as_ref().is_none_or(|e| pus_value(e, name, nums))
                && pus_stmt(body, name, nums)
        }
        Break { .. } | Continue { .. } => true,
        _ => false,
    }
}

/// An expression position for S-0: `name` may be an object-literal field value, a
/// numeric operand, or absent. Bare `name` at this level (an escape) bails.
fn pus_value(e: &Expr, name: &str, nums: &HashSet<String>) -> bool {
    use Expr::*;
    match e {
        // `{ k: name, … }` — the field-value use S-0 exists to permit. Each prop value
        // must itself be S-0-safe (so a nested `{ k: name + "!" }` still bails via nus).
        Object { props, .. } => props.iter().all(|p| match p {
            tishlang_ast::ObjectProp::KeyValue(_, v, _) => {
                // A bare `name` as a field value is OK; otherwise it must be numeric-safe.
                matches!(v, Expr::Ident { name: n, .. } if n.as_ref() == name)
                    || pus_value(v, name, nums)
            }
            tishlang_ast::ObjectProp::Spread(_) => false,
        }),
        // Anywhere else, defer to the numeric-operand rules (a bare `name` here is not a
        // numeric operand and `nus_expr` returns false for it → escape bails, as intended).
        _ => nus_expr(e, name, nums),
    }
}

/// Is `s` a fn body whose EVERY `return` returns the SAME object literal whose
/// fields are all `number` (under `pctx`, the params-as-number context)? Returns
/// the field list of that shape, or None.
fn return_object_shape(
    s: &Statement,
    pctx: &InferCtx,
) -> Option<Vec<(std::sync::Arc<str>, TypeAnnotation)>> {
    let mut found: Option<Vec<(std::sync::Arc<str>, TypeAnnotation)>> = None;
    if !collect_return_shapes(s, pctx, &mut found, &mut true) {
        return None;
    }
    found
}

/// Walk `s`; for each `return <obj>` confirm it's an all-f64 object literal with a
/// key set/order identical to any previously seen one. `ok` flips false on any
/// non-conforming return (a non-object return, a shape mismatch, an untypeable
/// field). Returns `ok`.
fn collect_return_shapes(
    s: &Statement,
    pctx: &InferCtx,
    found: &mut Option<Vec<(std::sync::Arc<str>, TypeAnnotation)>>,
    ok: &mut bool,
) -> bool {
    use Statement::*;
    if !*ok {
        return false;
    }
    match s {
        Return { value: Some(Expr::Object { props, .. }), .. } => {
            match infer_object_shape(props, pctx) {
                Some(fields) => {
                    // HARD all-f64 gate (string/bool field ⇒ bail to boxed).
                    if !fields.iter().all(|(_, t)| is_number(t)) {
                        *ok = false;
                        return false;
                    }
                    match found {
                        None => *found = Some(fields),
                        Some(prev) => {
                            // Identical ordered key set AND identical field types.
                            if prev.len() != fields.len()
                                || prev.iter().zip(&fields).any(|((pk, pt), (fk, ft))| {
                                    pk != fk || type_canon(pt) != type_canon(ft)
                                })
                            {
                                *ok = false;
                                return false;
                            }
                        }
                    }
                }
                None => {
                    *ok = false;
                    return false;
                }
            }
        }
        // A `return` of anything other than an object literal ⇒ not a struct factory.
        Return { .. } => {
            *ok = false;
            return false;
        }
        Block { statements, .. } | Multi { statements, .. } => {
            for x in statements {
                if !collect_return_shapes(x, pctx, found, ok) {
                    return false;
                }
            }
        }
        If { then_branch, else_branch, .. } => {
            if !collect_return_shapes(then_branch, pctx, found, ok) {
                return false;
            }
            if let Some(e) = else_branch {
                if !collect_return_shapes(e, pctx, found, ok) {
                    return false;
                }
            }
        }
        For { body, .. } | While { body, .. } | DoWhile { body, .. } | ForOf { body, .. } => {
            if !collect_return_shapes(body, pctx, found, ok) {
                return false;
            }
        }
        // No return here.
        _ => {}
    }
    *ok
}

/// Run the S-0..S-C analysis over a program (read-only; no mutation). Exposed for
/// unit tests and the future S-D..S-F consumers.
pub fn analyze_aggregate(program: &Program) -> AggregateAnalysis {
    let mut analysis = AggregateAnalysis::default();
    let mut reg = StructRegistry::default();

    // ---- S-0 + S-A: per top-level function, find numeric params and a return shape.
    for s in &program.statements {
        if let Statement::FunDecl { name, params, body, return_type: None, async_: false, rest_param: None, .. } = s {
            // Locals provably numeric in the body (so overloaded `+`/comparison resolve).
            let mut nums = HashSet::new();
            collect_numeric_locals_fixpoint(body, &mut nums);
            // S-0: a simple, unannotated, default-less param used only as a field value /
            // numerically is numeric. OPTIMISTIC FIXPOINT (mirrors M4 #172): assume ALL candidate
            // params numeric, verify each under that hypothesis, drop the failures, repeat until
            // stable. The surviving set is self-consistent — every overloaded `+`/comparison's
            // OTHER operand is itself a surviving numeric param — so a param that only resolves by
            // relying on a sibling that later bails is itself dropped (no `f64 + Value` miscompile).
            let candidates: HashSet<String> = params
                .iter()
                .filter_map(|p| match p {
                    FunParam::Simple(tp) if tp.type_ann.is_none() && tp.default.is_none() => {
                        Some(tp.name.to_string())
                    }
                    _ => None,
                })
                .collect();
            let mut numeric_params = candidates.clone();
            loop {
                let mut local_nums = nums.clone();
                for n in &numeric_params {
                    local_nums.insert(n.clone());
                }
                let drop: Vec<String> = numeric_params
                    .iter()
                    .filter(|n| !pus_stmt(body, n.as_str(), &local_nums))
                    .cloned()
                    .collect();
                if drop.is_empty() {
                    break;
                }
                for n in drop {
                    numeric_params.remove(&n);
                }
            }
            // Build a param-context: every S-0 numeric param is `: number`.
            let mut pctx = InferCtx::new();
            for p in params {
                if let FunParam::Simple(tp) = p {
                    if let Some(ann) = &tp.type_ann {
                        pctx.define(tp.name.as_ref(), ann.clone());
                    } else if numeric_params.contains(tp.name.as_ref()) {
                        pctx.define(tp.name.as_ref(), number_ann());
                    }
                }
            }
            if !numeric_params.is_empty() {
                analysis.fn_numeric_params.insert(name.to_string(), numeric_params);
            }
            // S-A: a single all-f64 object-literal return shape ⇒ a struct alias.
            if let Some(fields) = return_object_shape(body, &pctx) {
                let alias = reg.intern(&fields);
                analysis.fn_return.insert(name.to_string(), AggReturn::Struct(alias));
            }
        }
    }

    // ---- S-B (array return): a fn whose return is `[ident, …]` all of one struct type.
    // Needs the per-fn struct returns from S-A, plus the locals' struct types inside the body.
    for s in &program.statements {
        if let Statement::FunDecl { name, body, .. } = s {
            if analysis.fn_return.contains_key(name.as_ref()) {
                continue; // already a struct factory
            }
            if let Some(alias) = fn_returns_array_of_struct(body, &analysis) {
                analysis
                    .fn_return
                    .insert(name.to_string(), AggReturn::ArrayOfStruct(alias));
            }
        }
    }

    // ---- S-B/S-C (top level): propagate call-return + array-of-ident to top-level locals.
    let mut local_types: HashMap<String, AggReturn> = HashMap::new();
    for s in &program.statements {
        if let Statement::VarDecl { name, type_ann: None, init: Some(init), .. } = s {
            if let Some(t) = infer_aggregate_expr(init, &analysis, &local_types) {
                local_types.insert(name.to_string(), t);
            }
        }
    }
    analysis.local_agg = local_types;

    analysis.struct_decls = reg.decls;

    // ---- S-D: whole-program unboxing candidacy. All-or-nothing per struct alias.
    compute_unbox_candidacy(program, &mut analysis);

    analysis
}

/// S-D candidacy: decide whether a single struct alias may be FULLY unboxed into a
/// native `VmRef<Vec<TishStruct_alias>>` threaded through de-virtualized typed free
/// fns. Sets `analysis.unbox_alias`/`array_param_fns`/`array_locals` iff EVERY use of
/// the struct group is safe (else leaves them empty → boxed). Conservative and
/// all-or-nothing: a single unsafe use anywhere bails the whole alias.
fn compute_unbox_candidacy(program: &Program, analysis: &mut AggregateAnalysis) {
    // The factory: a fn whose return shape is `Struct(alias)`.
    // Require EXACTLY one struct alias overall (nbody shape) — keeps it monomorphic
    // and avoids ambiguity about which array elements are which struct.
    if analysis.struct_decls.len() != 1 {
        return;
    }
    let alias = analysis.struct_decls[0].0.clone();

    // There must be an array factory returning `ArrayOfStruct(alias)` (makeBodies),
    // and at least one top-level `let` of `ArrayOfStruct(alias)` (bodies).
    let has_array_factory = analysis
        .fn_return
        .values()
        .any(|r| matches!(r, AggReturn::ArrayOfStruct(a) if *a == alias));
    if !has_array_factory {
        return;
    }
    let array_locals: Vec<String> = analysis
        .local_agg
        .iter()
        .filter_map(|(n, r)| match r {
            AggReturn::ArrayOfStruct(a) if *a == alias => Some(n.clone()),
            _ => None,
        })
        .collect();
    if array_locals.is_empty() {
        return;
    }

    // Identify every fn whose first param is THE bodies array (used as `p[i]`,
    // `p.length`, `p[i].field` read/write). Each such fn must pass the per-fn
    // struct-array body safety predicate; a single failure bails the whole alias.
    let mut array_param_fns: HashMap<String, (usize, String)> = HashMap::new();
    for s in &program.statements {
        if let Statement::FunDecl {
            name, params, body, async_: false, rest_param: None, ..
        } = s
        {
            // Find a param used array-of-struct-ish.
            for (pi, p) in params.iter().enumerate() {
                let pname = match p {
                    FunParam::Simple(tp) if tp.default.is_none() => tp.name.as_ref(),
                    _ => continue,
                };
                if !param_used_as_struct_array(body, pname) {
                    continue;
                }
                // This param is the bodies array. The whole fn body must be unbox-safe
                // for that array (every element use is a literal-key field op; no escape,
                // no reshape, no `===`, no computed-key). Bail the whole alias otherwise.
                if !struct_array_fn_safe(body, pname) {
                    return;
                }
                array_param_fns.insert(name.to_string(), (pi, pname.to_string()));
                break; // one array param per fn (nbody shape)
            }
        }
    }
    if array_param_fns.is_empty() {
        return;
    }

    // Every CALL of an array-param fn must pass the array as a bare top-level array
    // local (no boxed reshaping). And the array local must not escape elsewhere
    // (passed to a non-group fn, stored, console.log'd, indexed-assigned wholesale).
    // The call-site routing + escape check is enforced in codegen too, but we gate
    // here so the annotations are only stamped when the program globally conforms.
    if !array_use_sites_safe(program, &array_locals, &array_param_fns) {
        return;
    }

    analysis.unbox_alias = Some(alias);
    analysis.array_param_fns = array_param_fns;
    analysis.array_locals = array_locals;
}

/// Does `body` use `p` in a struct-array shape — at least one `p[i]` index, `p.length`,
/// or `p[i].field` access? (Used to *identify* the array param, before safety-checking.)
fn param_used_as_struct_array(body: &Statement, p: &str) -> bool {
    let mut found = false;
    walk_exprs_stmt(body, &mut |e| {
        match e {
            Expr::Index { object, .. } => {
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p) {
                    found = true;
                }
            }
            Expr::Member { object, prop: tishlang_ast::MemberProp::Name { name: m, .. }, .. } => {
                if m.as_ref() == "length"
                    && matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p)
                {
                    found = true;
                }
            }
            _ => {}
        }
    });
    found
}

/// S-D per-fn struct-array safety. Every use of array param `p` inside `body` must be one of:
///   * `p.length`                        (read)
///   * `p[i]` as a `let x = p[i]` element-alias OR directly `p[i].field` read/write
///   * `p[i].field` read / `p[i].field = <scalar>` write
///   * `x.field` read / `x.field = <scalar>` write where `x` is an alias `let x = p[i]`
/// Anything else — bare `p`, `p` as a call arg, `p[i]` stored/escaped, computed `.field`,
/// `===`/`!==` on a body, `p.push`/`p.splice`/`p.length = …` reshape — bails (false).
fn struct_array_fn_safe(body: &Statement, p: &str) -> bool {
    // Collect element-alias locals: `let x = p[i]` where `i` is a simple ident.
    // Track alias name → index-var name. Require the index var is never reassigned
    // in the alias's scope and the alias never escapes (checked via uses).
    let mut aliases: HashSet<String> = HashSet::new();
    collect_element_aliases(body, p, &mut aliases);
    saf_stmt(body, p, &aliases)
}

/// Collect `let x = p[idx]` element-alias binding names (idx a bare ident).
fn collect_element_aliases(s: &Statement, p: &str, out: &mut HashSet<String>) {
    use Statement::*;
    match s {
        VarDecl { name, init: Some(init), .. } => {
            if let Expr::Index { object, index, .. } = init {
                if matches!(object.as_ref(), Expr::Ident { name: o, .. } if o.as_ref() == p)
                    && matches!(index.as_ref(), Expr::Ident { .. })
                {
                    out.insert(name.to_string());
                }
            }
        }
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().for_each(|x| collect_element_aliases(x, p, out))
        }
        If { then_branch, else_branch, .. } => {
            collect_element_aliases(then_branch, p, out);
            if let Some(e) = else_branch {
                collect_element_aliases(e, p, out);
            }
        }
        For { init, body, .. } => {
            if let Some(i) = init {
                collect_element_aliases(i, p, out);
            }
            collect_element_aliases(body, p, out);
        }
        While { body, .. } | DoWhile { body, .. } | ForOf { body, .. } => {
            collect_element_aliases(body, p, out)
        }
        _ => {}
    }
}

/// Statement-level S-D safety walk for array param `p` with element aliases `aliases`.
fn saf_stmt(s: &Statement, p: &str, aliases: &HashSet<String>) -> bool {
    use Statement::*;
    match s {
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().all(|x| saf_stmt(x, p, aliases))
        }
        VarDecl { name, init, .. } => {
            // `let x = p[i]` element-alias decl is OK (the alias is registered).
            // Any other decl: the init must be p-safe AND must not bind `p` itself.
            if name.as_ref() == p {
                return false;
            }
            if let Some(Expr::Index { object, index, .. }) = init {
                if matches!(object.as_ref(), Expr::Ident { name: o, .. } if o.as_ref() == p) {
                    // element alias: index must be a simple ident, and the binding must be
                    // in `aliases` (it is, by construction). OK.
                    return matches!(index.as_ref(), Expr::Ident { .. })
                        && aliases.contains(name.as_ref());
                }
            }
            init.as_ref().is_none_or(|e| saf_expr(e, p, aliases))
        }
        ExprStmt { expr, .. } => saf_expr(expr, p, aliases),
        Return { value, .. } => {
            // A bare `return p` escapes the array; a `return <scalar>` is fine.
            value.as_ref().is_none_or(|e| saf_expr(e, p, aliases))
        }
        If { cond, then_branch, else_branch, .. } => {
            saf_expr(cond, p, aliases)
                && saf_stmt(then_branch, p, aliases)
                && else_branch.as_ref().is_none_or(|e| saf_stmt(e, p, aliases))
        }
        While { cond, body, .. } => saf_expr(cond, p, aliases) && saf_stmt(body, p, aliases),
        DoWhile { cond, body, .. } => saf_expr(cond, p, aliases) && saf_stmt(body, p, aliases),
        For { init, cond, update, body, .. } => {
            init.as_ref().is_none_or(|x| saf_stmt(x, p, aliases))
                && cond.as_ref().is_none_or(|e| saf_expr(e, p, aliases))
                && update.as_ref().is_none_or(|e| saf_expr(e, p, aliases))
                && saf_stmt(body, p, aliases)
        }
        Break { .. } | Continue { .. } => true,
        _ => false,
    }
}

/// Expression-level S-D safety. `p` is the array param; `aliases` are `let x = p[i]`
/// element aliases. Returns false on any unsafe use of `p` or an alias.
fn saf_expr(e: &Expr, p: &str, aliases: &HashSet<String>) -> bool {
    use Expr::*;
    match e {
        Literal { .. } => true,
        Ident { name, .. } => {
            // A BARE reference to the array `p` (not behind `[]`/`.length`) escapes — bail.
            // A bare reference to an element alias `x` ALSO escapes (the alias must only
            // appear as `x.field`); bail so the alias never leaks as a value.
            name.as_ref() != p && !aliases.contains(name.as_ref())
        }
        // `p.length` (read) — OK. `p.field` for any other field is not a thing on the array;
        // bail. `x.field` where `x` is an element alias — OK (Copy f64 read).
        Member { object, prop, optional: false, .. } => {
            let mname = match prop {
                tishlang_ast::MemberProp::Name { name, .. } => name.as_ref(),
                tishlang_ast::MemberProp::Expr(_) => return false, // computed key bails
            };
            match object.as_ref() {
                Ident { name, .. } if name.as_ref() == p => mname == "length",
                Ident { name, .. } if aliases.contains(name.as_ref()) => true,
                // `p[i].field` read.
                Index { object: io, index, .. } => {
                    matches!(io.as_ref(), Ident { name, .. } if name.as_ref() == p)
                        && saf_expr(index, p, aliases)
                }
                _ => saf_expr(object, p, aliases),
            }
        }
        Member { .. } => false, // optional member on the array/alias bails
        // `p[i]` index: only valid as part of `p[i].field` (handled above) or `let x = p[i]`
        // (handled in saf_stmt). A bare `p[i]` value here escapes the element — bail.
        Index { object, .. } => {
            if matches!(object.as_ref(), Ident { name, .. } if name.as_ref() == p) {
                return false;
            }
            // index into something else (not the array) — safe if its parts are safe.
            saf_expr(object, p, aliases)
        }
        Binary { left, op, right, .. } => {
            // `===`/`!==` directly on a body element / alias would compare references — bail.
            if matches!(op, BinOp::StrictEq | BinOp::StrictNe | BinOp::Eq | BinOp::Ne) {
                if expr_is_body_ref(left, p, aliases) || expr_is_body_ref(right, p, aliases) {
                    return false;
                }
            }
            saf_expr(left, p, aliases) && saf_expr(right, p, aliases)
        }
        Unary { operand, .. } => saf_expr(operand, p, aliases),
        Conditional { cond, then_branch, else_branch, .. } => {
            saf_expr(cond, p, aliases)
                && saf_expr(then_branch, p, aliases)
                && saf_expr(else_branch, p, aliases)
        }
        Assign { name, value, .. } => {
            name.as_ref() != p && !aliases.contains(name.as_ref()) && saf_expr(value, p, aliases)
        }
        CompoundAssign { name, value, .. } => {
            name.as_ref() != p && !aliases.contains(name.as_ref()) && saf_expr(value, p, aliases)
        }
        // Field write: `x.field = <scalar>` (alias) or `p[i].field = <scalar>` — OK iff RHS p-safe
        // (scalar). Computed-key write or write onto the bare array bails.
        MemberAssign { object, prop: _, value, .. } => {
            let target_ok = match object.as_ref() {
                Ident { name, .. } if aliases.contains(name.as_ref()) => true,
                Index { object: io, index, .. } => {
                    matches!(io.as_ref(), Ident { name, .. } if name.as_ref() == p)
                        && saf_expr(index, p, aliases)
                }
                _ => false,
            };
            target_ok && saf_expr(value, p, aliases)
        }
        // `p[i] = …` / `p.length = …` reshape, or any index-assign onto the array, bails.
        IndexAssign { object, .. } => {
            !matches!(object.as_ref(), Ident { name, .. } if name.as_ref() == p)
        }
        Call { callee, args, .. } => {
            // Method calls on the array (push/splice/etc.) or on an alias bail: `p.push(...)`,
            // `x.something()`. Math.<fn>(...) and other free calls are fine if args are p-safe
            // (and no body ref escapes as an arg — checked by saf_expr on each arg).
            if let Member { object, .. } = callee.as_ref() {
                if expr_is_body_ref(object, p, aliases) {
                    return false; // method call on the array/element
                }
            }
            saf_expr(callee, p, aliases)
                && args.iter().all(|a| match a {
                    CallArg::Expr(x) => saf_expr(x, p, aliases),
                    CallArg::Spread(_) => false,
                })
        }
        TemplateLiteral { exprs, .. } => exprs.iter().all(|x| saf_expr(x, p, aliases)),
        // Any other expr form that could touch `p` is not modelled — be conservative.
        PostfixInc { name, .. } | PostfixDec { name, .. } | PrefixInc { name, .. }
        | PrefixDec { name, .. } => name.as_ref() != p && !aliases.contains(name.as_ref()),
        _ => false,
    }
}

/// Is `e` a direct reference to the array `p` or an element alias (`x` or `p[i]`)?
fn expr_is_body_ref(e: &Expr, p: &str, aliases: &HashSet<String>) -> bool {
    match e {
        Expr::Ident { name, .. } => name.as_ref() == p || aliases.contains(name.as_ref()),
        Expr::Index { object, .. } => {
            matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p)
        }
        _ => false,
    }
}

/// The top-level program's uses of the array locals must be safe: each is constructed
/// once by an array factory call, only passed to group fns (or read scalar-only), and
/// never escapes (no console.log of the array, no store, no bare pass to a non-group fn).
fn array_use_sites_safe(
    program: &Program,
    array_locals: &[String],
    group_fns: &HashMap<String, (usize, String)>,
) -> bool {
    let set: HashSet<&str> = array_locals.iter().map(|s| s.as_str()).collect();
    let mut ok = true;
    for s in &program.statements {
        // Skip the decl of the array local itself.
        if let Statement::VarDecl { name, .. } = s {
            if set.contains(name.as_ref()) {
                continue;
            }
        }
        if !top_use_safe(s, &set, group_fns) {
            ok = false;
            break;
        }
    }
    ok
}

fn top_use_safe(
    s: &Statement,
    set: &HashSet<&str>,
    group_fns: &HashMap<String, (usize, String)>,
) -> bool {
    use Statement::*;
    match s {
        FunDecl { .. } | TypeAlias { .. } | DeclareVar { .. } | DeclareFun { .. } => true,
        VarDecl { init, .. } => init.as_ref().is_none_or(|e| top_use_expr(e, set, group_fns)),
        ExprStmt { expr, .. } => top_use_expr(expr, set, group_fns),
        For { init, cond, update, body, .. } => {
            init.as_ref().is_none_or(|x| top_use_safe(x, set, group_fns))
                && cond.as_ref().is_none_or(|e| top_use_expr(e, set, group_fns))
                && update.as_ref().is_none_or(|e| top_use_expr(e, set, group_fns))
                && top_use_safe(body, set, group_fns)
        }
        While { cond, body, .. } | DoWhile { cond, body, .. } => {
            top_use_expr(cond, set, group_fns) && top_use_safe(body, set, group_fns)
        }
        If { cond, then_branch, else_branch, .. } => {
            top_use_expr(cond, set, group_fns)
                && top_use_safe(then_branch, set, group_fns)
                && else_branch.as_ref().is_none_or(|e| top_use_safe(e, set, group_fns))
        }
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().all(|x| top_use_safe(x, set, group_fns))
        }
        Return { value, .. } => value.as_ref().is_none_or(|e| top_use_expr(e, set, group_fns)),
        _ => true,
    }
}

fn top_use_expr(
    e: &Expr,
    set: &HashSet<&str>,
    group_fns: &HashMap<String, (usize, String)>,
) -> bool {
    use Expr::*;
    match e {
        // A bare reference to an array local escapes UNLESS it's a call arg to a group fn
        // at the expected array-param position (handled in Call below).
        Ident { name, .. } => !set.contains(name.as_ref()),
        Call { callee, args, .. } => {
            // A call to a group fn may pass an array local at its array-param slot.
            if let Ident { name: fname, .. } = callee.as_ref() {
                if let Some((api, _)) = group_fns.get(fname.as_ref()) {
                    return args.iter().enumerate().all(|(ai, a)| match a {
                        CallArg::Expr(Ident { name, .. }) if ai == *api => {
                            // array local at the array slot — OK; or a non-array ident.
                            set.contains(name.as_ref()) || true
                        }
                        CallArg::Expr(x) => top_use_expr(x, set, group_fns),
                        CallArg::Spread(_) => false,
                    });
                }
            }
            top_use_expr(callee, set, group_fns)
                && args.iter().all(|a| match a {
                    CallArg::Expr(x) => top_use_expr(x, set, group_fns),
                    CallArg::Spread(_) => false,
                })
        }
        // `arr[i]` / `arr.length` at top level reads scalars — that's a boxed read; but the
        // array local should only be consumed via group fns. Reading `arr.length` etc. at top
        // level is rare and would require the boxed array; bail to keep semantics simple.
        Index { object, .. } => !expr_refs_set(object, set) && top_use_expr(object, set, group_fns),
        Member { object, .. } => !expr_refs_set(object, set) && top_use_expr(object, set, group_fns),
        Binary { left, right, .. } => {
            top_use_expr(left, set, group_fns) && top_use_expr(right, set, group_fns)
        }
        Unary { operand, .. } => top_use_expr(operand, set, group_fns),
        Conditional { cond, then_branch, else_branch, .. } => {
            top_use_expr(cond, set, group_fns)
                && top_use_expr(then_branch, set, group_fns)
                && top_use_expr(else_branch, set, group_fns)
        }
        Assign { value, .. } => top_use_expr(value, set, group_fns),
        CompoundAssign { value, .. } => top_use_expr(value, set, group_fns),
        TemplateLiteral { exprs, .. } => exprs.iter().all(|x| top_use_expr(x, set, group_fns)),
        Literal { .. } => true,
        Array { elements, .. } => elements.iter().all(|el| match el {
            tishlang_ast::ArrayElement::Expr(x) => top_use_expr(x, set, group_fns),
            tishlang_ast::ArrayElement::Spread(_) => false,
        }),
        PostfixInc { .. } | PostfixDec { .. } | PrefixInc { .. } | PrefixDec { .. } => true,
        _ => true,
    }
}

fn expr_refs_set(e: &Expr, set: &HashSet<&str>) -> bool {
    matches!(e, Expr::Ident { name, .. } if set.contains(name.as_ref()))
}

/// Walk every sub-expression of a statement, invoking `f` on each.
fn walk_exprs_stmt(s: &Statement, f: &mut impl FnMut(&Expr)) {
    use Statement::*;
    match s {
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().for_each(|x| walk_exprs_stmt(x, f))
        }
        VarDecl { init, .. } => {
            if let Some(e) = init {
                walk_exprs_expr(e, f);
            }
        }
        VarDeclDestructure { init, .. } => walk_exprs_expr(init, f),
        ExprStmt { expr, .. } => walk_exprs_expr(expr, f),
        Return { value, .. } => {
            if let Some(e) = value {
                walk_exprs_expr(e, f);
            }
        }
        If { cond, then_branch, else_branch, .. } => {
            walk_exprs_expr(cond, f);
            walk_exprs_stmt(then_branch, f);
            if let Some(e) = else_branch {
                walk_exprs_stmt(e, f);
            }
        }
        While { cond, body, .. } | DoWhile { cond, body, .. } => {
            walk_exprs_expr(cond, f);
            walk_exprs_stmt(body, f);
        }
        For { init, cond, update, body, .. } => {
            if let Some(i) = init {
                walk_exprs_stmt(i, f);
            }
            if let Some(c) = cond {
                walk_exprs_expr(c, f);
            }
            if let Some(u) = update {
                walk_exprs_expr(u, f);
            }
            walk_exprs_stmt(body, f);
        }
        ForOf { iterable, body, .. } => {
            walk_exprs_expr(iterable, f);
            walk_exprs_stmt(body, f);
        }
        Throw { value, .. } => walk_exprs_expr(value, f),
        _ => {}
    }
}

fn walk_exprs_expr(e: &Expr, f: &mut impl FnMut(&Expr)) {
    use Expr::*;
    f(e);
    match e {
        Binary { left, right, .. } => {
            walk_exprs_expr(left, f);
            walk_exprs_expr(right, f);
        }
        Unary { operand, .. } | TypeOf { operand, .. } | Await { operand, .. }
        | Delete { target: operand, .. } => walk_exprs_expr(operand, f),
        Call { callee, args, .. } | New { callee, args, .. } => {
            walk_exprs_expr(callee, f);
            for a in args {
                match a {
                    CallArg::Expr(x) | CallArg::Spread(x) => walk_exprs_expr(x, f),
                }
            }
        }
        Member { object, prop, .. } => {
            walk_exprs_expr(object, f);
            if let tishlang_ast::MemberProp::Expr(p) = prop {
                walk_exprs_expr(p, f);
            }
        }
        Index { object, index, .. } => {
            walk_exprs_expr(object, f);
            walk_exprs_expr(index, f);
        }
        Conditional { cond, then_branch, else_branch, .. } => {
            walk_exprs_expr(cond, f);
            walk_exprs_expr(then_branch, f);
            walk_exprs_expr(else_branch, f);
        }
        NullishCoalesce { left, right, .. } => {
            walk_exprs_expr(left, f);
            walk_exprs_expr(right, f);
        }
        Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(x)
                    | tishlang_ast::ArrayElement::Spread(x) => walk_exprs_expr(x, f),
                }
            }
        }
        Object { props, .. } => {
            for p in props {
                match p {
                    tishlang_ast::ObjectProp::KeyValue(_, v, _) => walk_exprs_expr(v, f),
                    tishlang_ast::ObjectProp::Spread(x) => walk_exprs_expr(x, f),
                }
            }
        }
        Assign { value, .. } | CompoundAssign { value, .. } | LogicalAssign { value, .. } => {
            walk_exprs_expr(value, f)
        }
        MemberAssign { object, value, .. } => {
            walk_exprs_expr(object, f);
            walk_exprs_expr(value, f);
        }
        IndexAssign { object, index, value, .. } => {
            walk_exprs_expr(object, f);
            walk_exprs_expr(index, f);
            walk_exprs_expr(value, f);
        }
        TemplateLiteral { exprs, .. } => exprs.iter().for_each(|x| walk_exprs_expr(x, f)),
        _ => {}
    }
}

/// S-B array-return: `return [a, b, …]` where each element is a bare ident of the
/// SAME `Struct(alias)` (resolved from the locals defined in this body via call-
/// return). Returns the alias if so.
fn fn_returns_array_of_struct(body: &Statement, analysis: &AggregateAnalysis) -> Option<String> {
    // Map this body's locals to struct aliases (only `let x = factory(…)` shapes).
    let mut locals: HashMap<String, String> = HashMap::new();
    collect_body_struct_locals(body, analysis, &mut locals);
    // Find the (single) `return [idents]`.
    find_array_return(body, &locals)
}

fn collect_body_struct_locals(
    s: &Statement,
    analysis: &AggregateAnalysis,
    out: &mut HashMap<String, String>,
) {
    use Statement::*;
    match s {
        VarDecl { name, type_ann: None, init: Some(init), .. } => {
            if let Some(AggReturn::Struct(alias)) =
                infer_aggregate_expr(init, analysis, &HashMap::new())
            {
                out.insert(name.to_string(), alias);
            }
        }
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().for_each(|x| collect_body_struct_locals(x, analysis, out))
        }
        If { then_branch, else_branch, .. } => {
            collect_body_struct_locals(then_branch, analysis, out);
            if let Some(e) = else_branch {
                collect_body_struct_locals(e, analysis, out);
            }
        }
        For { body, .. } | While { body, .. } | DoWhile { body, .. } | ForOf { body, .. } => {
            collect_body_struct_locals(body, analysis, out)
        }
        _ => {}
    }
}

fn find_array_return(s: &Statement, locals: &HashMap<String, String>) -> Option<String> {
    use Statement::*;
    match s {
        Return { value: Some(Expr::Array { elements, .. }), .. } => {
            let mut alias: Option<String> = None;
            for el in elements {
                let e = match el {
                    tishlang_ast::ArrayElement::Expr(e) => e,
                    tishlang_ast::ArrayElement::Spread(_) => return None,
                };
                let a = match e {
                    Expr::Ident { name, .. } => locals.get(name.as_ref())?,
                    _ => return None,
                };
                match &alias {
                    None => alias = Some(a.clone()),
                    Some(prev) if prev != a => return None,
                    _ => {}
                }
            }
            alias
        }
        Block { statements, .. } | Multi { statements, .. } => {
            statements.iter().find_map(|x| find_array_return(x, locals))
        }
        If { then_branch, else_branch, .. } => find_array_return(then_branch, locals)
            .or_else(|| else_branch.as_ref().and_then(|e| find_array_return(e, locals))),
        For { body, .. } | While { body, .. } | DoWhile { body, .. } | ForOf { body, .. } => {
            find_array_return(body, locals)
        }
        _ => None,
    }
}

/// S-B/S-C: the aggregate type of an init expression.
///   * `factory(…)` where `factory` has an S-A/S-B return shape → that shape.
///   * `[a, b, …]` where every element is a struct-typed local → `ArrayOfStruct`.
fn infer_aggregate_expr(
    e: &Expr,
    analysis: &AggregateAnalysis,
    locals: &HashMap<String, AggReturn>,
) -> Option<AggReturn> {
    match e {
        // S-B: call-return propagation.
        Expr::Call { callee, .. } => {
            if let Expr::Ident { name, .. } = callee.as_ref() {
                return analysis.fn_return.get(name.as_ref()).cloned();
            }
            None
        }
        // S-C: array-of-ident element typing.
        Expr::Array { elements, .. } => {
            let mut alias: Option<String> = None;
            for el in elements {
                let x = match el {
                    tishlang_ast::ArrayElement::Expr(x) => x,
                    tishlang_ast::ArrayElement::Spread(_) => return None,
                };
                let a = match x {
                    Expr::Ident { name, .. } => match locals.get(name.as_ref()) {
                        Some(AggReturn::Struct(a)) => a.clone(),
                        _ => return None,
                    },
                    Expr::Call { .. } => match infer_aggregate_expr(x, analysis, locals) {
                        Some(AggReturn::Struct(a)) => a,
                        _ => return None,
                    },
                    _ => return None,
                };
                match &alias {
                    None => alias = Some(a),
                    Some(prev) if *prev != a => return None,
                    _ => {}
                }
            }
            alias.map(AggReturn::ArrayOfStruct)
        }
        // A bare ident copy of a struct-typed local.
        Expr::Ident { name, .. } => locals.get(name.as_ref()).cloned(),
        _ => None,
    }
}

/// Aggregate inference writeback. Runs the S-0..S-C analysis, then stamps onto the
/// program ONLY the annotations the existing codegen backs soundly (see the module
/// note above): the S-0 numeric `: number` params, plus the inert struct alias
/// `type` decls. The S-A/S-B/S-C struct shapes are computed and available via
/// `analyze_aggregate` but are NOT written onto params / call-locals until the
/// S-D..S-F typed-fn ABI tier exists to consume them without a boxed-edge miscompile.
fn aggregate_infer_program(program: Program) -> Program {
    let analysis = analyze_aggregate(&program);

    // Stamp S-0 numeric params onto the matching FunDecls. When S-D candidacy holds
    // (`unbox_alias` is `Some`), ALSO stamp the struct/array annotations: the factory
    // return `: alias`, the array-factory return `: alias[]`, the group fns' array
    // params `: alias[]`, and the top-level array local(s) `: alias[]`. Codegen's S-F
    // tier consumes these (and bypasses the boxed closure path) so the writes persist.
    let mut statements: Vec<Statement> = program
        .statements
        .into_iter()
        .map(|s| stamp_aggregate(s, &analysis))
        .collect();

    // Prepend the struct alias decls. Under S-D candidacy codegen canonicalises the
    // matching alias into `RustType::Named` and emits a `Copy` struct; otherwise the
    // alias is inert (unreferenced aliases are dropped downstream).
    if !analysis.struct_decls.is_empty() {
        let span = statements.first().map(stmt_span).unwrap_or_else(zero_span);
        let mut out: Vec<Statement> = Vec::with_capacity(statements.len() + analysis.struct_decls.len());
        for (name, fields) in &analysis.struct_decls {
            out.push(Statement::TypeAlias {
                name: name.as_str().into(),
                name_span: span,
                ty: TypeAnnotation::Object(fields.clone()),
                span,
            });
        }
        out.append(&mut statements);
        statements = out;
    }

    Program { statements }
}

/// `alias[]` annotation (an `Array(Simple(alias))`), the S-D array-param/local type.
fn array_of_alias_ann(alias: &str) -> TypeAnnotation {
    TypeAnnotation::Array(Box::new(TypeAnnotation::Simple(alias.into(), zero_span())))
}

/// Stamp both the S-0 numeric scalar params AND, under S-D candidacy, the struct/array
/// annotations onto a top-level statement.
fn stamp_aggregate(s: Statement, analysis: &AggregateAnalysis) -> Statement {
    // First apply the always-safe S-0 numeric-param stamping.
    let s = stamp_numeric_params(s, analysis);
    let Some(alias) = analysis.unbox_alias.as_deref() else {
        return s; // no S-D candidacy → only S-0 params stamped
    };
    match s {
        // Stamp fn returns / array params for the unbox group.
        Statement::FunDecl {
            async_, name, name_span, params, rest_param, return_type, body, span,
        } => {
            // Return type: factory → `: alias`; array-factory → `: alias[]`.
            let new_return = match analysis.fn_return.get(name.as_ref()) {
                Some(AggReturn::Struct(a)) if a == alias => {
                    Some(TypeAnnotation::Simple(alias.into(), zero_span()))
                }
                Some(AggReturn::ArrayOfStruct(a)) if a == alias => Some(array_of_alias_ann(alias)),
                _ => return_type,
            };
            // Array param: the group fn's bodies param → `: alias[]`.
            let array_pi = analysis.array_param_fns.get(name.as_ref()).map(|(pi, _)| *pi);
            let new_params: Vec<FunParam> = params
                .into_iter()
                .enumerate()
                .map(|(pi, p)| match p {
                    FunParam::Simple(mut tp) if Some(pi) == array_pi => {
                        tp.type_ann = Some(array_of_alias_ann(alias));
                        FunParam::Simple(tp)
                    }
                    other => other,
                })
                .collect();
            Statement::FunDecl {
                async_, name, name_span, params: new_params, rest_param,
                return_type: new_return, body, span,
            }
        }
        // Stamp the top-level array local(s) `: alias[]`.
        Statement::VarDecl { name, name_span, mutable, type_ann, init, span }
            if analysis.array_locals.iter().any(|n| n == name.as_ref()) =>
        {
            Statement::VarDecl {
                name, name_span, mutable,
                type_ann: type_ann.or_else(|| Some(array_of_alias_ann(alias))),
                init, span,
            }
        }
        other => other,
    }
}

fn stamp_numeric_params(s: Statement, analysis: &AggregateAnalysis) -> Statement {
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
        let numeric = analysis.fn_numeric_params.get(name.as_ref());
        let new_params = params
            .into_iter()
            .map(|p| match p {
                FunParam::Simple(mut tp)
                    if tp.type_ann.is_none()
                        && tp.default.is_none()
                        && numeric.is_some_and(|set| set.contains(tp.name.as_ref())) =>
                {
                    tp.type_ann = Some(number_ann());
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
                                    TypeAnnotation::Simple(s, _) => s.to_string(),
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

    #[test]
    fn infers_param_copied_to_local_then_compared() {
        // #172 fannkuch keystone: the param `n` is copied into `r` (`let r = n`) and later compared
        // against it (`r === n`). The optimistic per-param fixpoint assumes `n` numeric, propagates
        // the copy so `r` is numeric, and `r === n` then proves `n` numeric (the other operand `r`
        // is provable). Closes the chicken-and-egg that previously left `n`/`r`/`count` boxed.
        let src =
            "fn f(n) { let r = n; while (r !== 1) { r = r - 1 }; return r === n }";
        assert_eq!(inferred_param(src, "f", "n").as_deref(), Some("number"));
    }

    #[test]
    fn does_not_infer_param_copied_then_string_concatenated() {
        // NEGATIVE (soundness): the copy `let r = x` flows into `+` against a string literal. The
        // overloaded-`+` rule still bails on `r + "!"` for the candidate... but more directly, `x`
        // itself is used in a non-numeric `+`, so the param MUST stay dynamic even though it is
        // copied to a local — the copy relaxation must not mask a genuinely non-numeric use.
        let src = "fn g(x) { let r = x; return r + \"!\" }";
        assert_eq!(inferred_param(src, "g", "x"), None);
    }
}

#[cfg(test)]
mod struct_infer_writeback_tests {
    use super::*;
    use tishlang_parser::parse;

    /// Annotation name written onto top-level `let <var>` after base inference + struct inference.
    fn local_ann(src: &str, var: &str) -> Option<String> {
        let parsed = parse(src).unwrap();
        let base = Program {
            statements: infer_statements(&parsed.statements, &mut InferCtx::new()),
        };
        let prog = struct_infer_program(base);
        for s in &prog.statements {
            if let Statement::VarDecl { name, type_ann, .. } = s {
                if name.as_ref() == var {
                    return type_ann.as_ref().map(|a| match a {
                        TypeAnnotation::Simple(s, _) => s.to_string(),
                        _ => "<complex>".to_string(),
                    });
                }
            }
        }
        None
    }

    #[test]
    fn writes_native_scalar_back_from_typed_array_index() {
        // #170: once `perm: number[]` is known, `let k = perm[0]` must carry `type_ann: number` on
        // the NODE (codegen reads the node, not the inference ctx) so it lowers to a native f64.
        let src = "let perm = [3, 2, 1, 0]\nlet k = perm[0]\nconsole.log(k)";
        assert_eq!(local_ann(src, "k").as_deref(), Some("number"));
    }

    #[test]
    fn does_not_annotate_non_native_local() {
        // The write-back must NEVER turn a non-number local into `number` (the base pass already
        // types this as `string`; the invariant is that the #170 write-back can't clobber it).
        let src = "let s = \"hi\"\nconsole.log(s)";
        assert_ne!(local_ann(src, "s").as_deref(), Some("number"));
    }
}

#[cfg(test)]
mod aggregate_infer_tests {
    use super::*;
    use tishlang_parser::parse;

    /// Parse + run base inference (so locals get their numeric types), then the aggregate analysis.
    fn analyze(src: &str) -> AggregateAnalysis {
        let parsed = parse(src).unwrap();
        let base = Program {
            statements: infer_statements(&parsed.statements, &mut InferCtx::new()),
        };
        analyze_aggregate(&base)
    }

    /// The inferred `: number` param annotations of `fn` after the full aggregate WRITEBACK.
    fn numeric_params_after_writeback(src: &str, fn_name: &str) -> Vec<String> {
        let parsed = parse(src).unwrap();
        let base = Program {
            statements: infer_statements(&parsed.statements, &mut InferCtx::new()),
        };
        let prog = aggregate_infer_program(base);
        let mut out = Vec::new();
        for s in &prog.statements {
            if let Statement::FunDecl { name, params, .. } = s {
                if name.as_ref() == fn_name {
                    for p in params {
                        if let FunParam::Simple(tp) = p {
                            if tp.type_ann.as_ref().is_some_and(is_number) {
                                out.push(tp.name.to_string());
                            }
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }

    // ---- S-0 -------------------------------------------------------------

    #[test]
    fn s0_infers_field_value_only_params_numeric() {
        // `body`'s params are used ONLY as object-literal field values — the case M4 cannot
        // handle (object store is not numeric-safe in `nus_*`). S-0 must type all seven numeric.
        let src = "function body(x, y, z, vx, vy, vz, mass) {\n  return { x: x, y: y, z: z, vx: vx, vy: vy, vz: vz, mass: mass }\n}";
        let a = analyze(src);
        let set = a.fn_numeric_params.get("body").cloned().unwrap_or_default();
        let mut got: Vec<_> = set.into_iter().collect();
        got.sort();
        assert_eq!(got, vec!["mass", "vx", "vy", "vz", "x", "y", "z"]);
    }

    #[test]
    fn s0_mixed_field_value_and_arithmetic_param() {
        // A param used as a field value AND in arithmetic is still numeric.
        let src = "function f(a, b) { return { sum: a + b, a: a } }";
        let a = analyze(src);
        let set = a.fn_numeric_params.get("f").cloned().unwrap_or_default();
        assert!(set.contains("a") && set.contains("b"));
    }

    #[test]
    fn s0_bails_on_string_concat_field_value() {
        // `{ label: "v=" + p }` is string concat — NOT numeric; the param must stay dynamic.
        let src = "function f(p) { return { label: \"v=\" + p } }";
        let a = analyze(src);
        assert!(!a.fn_numeric_params.get("f").map(|s| s.contains("p")).unwrap_or(false));
    }

    #[test]
    fn s0_optimistic_fixpoint_drops_string_sibling() {
        // S-0's ALL-params optimistic fixpoint must still drop a genuinely non-numeric param even
        // when a SIBLING is numeric: `f(a, b)` with `a` a numeric loop bound but `b` string-
        // concatenated. `a` is typed; `b` is NOT (else `"r="+b` would mistype `b` to f64). This is
        // the joint-dependency soundness guard (the fixpoint drops `b`, re-verifies, `a` survives).
        let src = "function f(a, b) {\n  let total = 0\n  for (let i = 0; i < a; i = i + 1) { total = total + i }\n  return \"r=\" + b\n}";
        let a = analyze(src);
        let set = a.fn_numeric_params.get("f").cloned().unwrap_or_default();
        assert!(set.contains("a"), "numeric loop-bound param must be typed");
        assert!(!set.contains("b"), "string-concatenated param must NOT be typed");
    }

    #[test]
    fn s0_bails_on_escaping_param() {
        // A param passed BARE to another call escapes (callee type unknown) — bail.
        let src = "function f(p) { return { boxed: g(p) } }\nfunction g(q) { return q }";
        let a = analyze(src);
        assert!(!a.fn_numeric_params.get("f").map(|s| s.contains("p")).unwrap_or(false));
    }

    // ---- S-A -------------------------------------------------------------

    #[test]
    fn sa_registers_return_struct_shape() {
        // `body()` returns one all-f64 object literal ⇒ a `Struct(alias)` return shape.
        let src = "function body(x, y, mass) { return { x: x, y: y, mass: mass } }";
        let a = analyze(src);
        assert!(matches!(a.fn_return.get("body"), Some(AggReturn::Struct(_))));
        // exactly one struct alias registered, with the three f64 fields in order.
        assert_eq!(a.struct_decls.len(), 1);
        let (_, fields) = &a.struct_decls[0];
        let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_ref()).collect();
        assert_eq!(keys, vec!["x", "y", "mass"]);
        assert!(fields.iter().all(|(_, t)| is_number(t)));
    }

    #[test]
    fn sa_bails_on_string_field_return_shape() {
        // HARD all-f64 gate: a string field ⇒ NOT a struct factory (S-D/S-F write lowering
        // would Copy-assume a non-Copy field). Must produce no return shape.
        let src = "function mk(n) { return { id: n, name: \"x\" } }";
        let a = analyze(src);
        assert!(a.fn_return.get("mk").is_none());
    }

    #[test]
    fn sa_bails_on_divergent_return_shapes() {
        // Two different shapes (different key sets) ⇒ not monomorphic ⇒ no return shape.
        let src = "function mk(c, a) { if (c > 0) { return { a: a } } return { b: a } }";
        let a = analyze(src);
        assert!(a.fn_return.get("mk").is_none());
    }

    // ---- S-B / S-C -------------------------------------------------------

    #[test]
    fn sb_propagates_call_return_to_local() {
        // `let p = body(…)` ⇒ the local has the `body` return struct shape.
        let src = "function body(x, y) { return { x: x, y: y } }\nlet p = body(1, 2)";
        let a = analyze(src);
        assert!(matches!(a.local_agg.get("p"), Some(AggReturn::Struct(_))));
    }

    #[test]
    fn sb_array_return_is_array_of_struct() {
        // `makeBodies()` returns `[sun, jupiter]` (both `body()` locals) ⇒ ArrayOfStruct, and a
        // top-level `let bodies = makeBodies()` carries that array-of-struct type.
        let src = "function body(x) { return { x: x } }\n\
                   function makeBodies() { let sun = body(1); let jup = body(2); return [sun, jup] }\n\
                   let bodies = makeBodies()";
        let a = analyze(src);
        assert!(matches!(a.fn_return.get("makeBodies"), Some(AggReturn::ArrayOfStruct(_))));
        assert!(matches!(a.local_agg.get("bodies"), Some(AggReturn::ArrayOfStruct(_))));
    }

    #[test]
    fn sc_array_of_struct_idents() {
        // A direct top-level `[a, b]` of struct-typed locals ⇒ ArrayOfStruct.
        let src = "function body(x) { return { x: x } }\n\
                   let a = body(1)\nlet b = body(2)\nlet arr = [a, b]";
        let a = analyze(src);
        assert!(matches!(a.local_agg.get("arr"), Some(AggReturn::ArrayOfStruct(_))));
    }

    #[test]
    fn sc_bails_on_heterogeneous_array() {
        // `[a, b]` where the two structs differ (different shapes) ⇒ no array-of-struct type.
        let src = "function p(x) { return { x: x } }\n\
                   function q(y) { return { y: y } }\n\
                   let a = p(1)\nlet b = q(2)\nlet arr = [a, b]";
        let a = analyze(src);
        assert!(a.local_agg.get("arr").is_none());
    }

    // ---- end-to-end: the nbody factory chain ----------------------------

    #[test]
    fn nbody_factory_chain_resolves() {
        // The actual nbody front-end shape: body() factory, makeBodies() array, bodies local.
        let src = "\
            function body(x, y, z, vx, vy, vz, mass) {\n\
              return { x: x, y: y, z: z, vx: vx, vy: vy, vz: vz, mass: mass }\n\
            }\n\
            function makeBodies() {\n\
              let sun = body(0, 0, 0, 0, 0, 0, 1)\n\
              let jup = body(1, 1, 1, 1, 1, 1, 1)\n\
              return [sun, jup]\n\
            }\n\
            let bodies = makeBodies()";
        let a = analyze(src);
        // S-0: all body params numeric.
        assert_eq!(a.fn_numeric_params.get("body").map(|s| s.len()), Some(7));
        // S-A: body is a struct factory.
        assert!(matches!(a.fn_return.get("body"), Some(AggReturn::Struct(_))));
        // S-B: makeBodies returns array-of-struct; `bodies` carries it.
        assert!(matches!(a.fn_return.get("makeBodies"), Some(AggReturn::ArrayOfStruct(_))));
        assert!(matches!(a.local_agg.get("bodies"), Some(AggReturn::ArrayOfStruct(_))));
    }

    // ---- writeback safety: only S-0 scalar params are stamped ------------

    #[test]
    fn writeback_stamps_only_s0_numeric_params() {
        // The WRITEBACK must add `: number` to body's params (soundly consumed) and must NOT
        // turn any param into a struct/array annotation (which codegen can't yet back).
        let src = "function body(x, y, z, vx, vy, vz, mass) {\n  return { x: x, y: y, z: z, vx: vx, vy: vy, vz: vz, mass: mass }\n}\nlet b = body(1,2,3,4,5,6,7)";
        let got = numeric_params_after_writeback(src, "body");
        assert_eq!(got, vec!["mass", "vx", "vy", "vz", "x", "y", "z"]);
    }

    #[test]
    fn writeback_does_not_stamp_call_local_with_struct() {
        // `let b = body(…)` must remain UN-annotated after writeback — stamping a `Named`
        // annotation here would miscompile (the call still returns a boxed Value). Guards the
        // soundness boundary until S-F lands.
        let src = "function body(x) { return { x: x } }\nlet b = body(1)";
        let parsed = parse(src).unwrap();
        let base = Program {
            statements: infer_statements(&parsed.statements, &mut InferCtx::new()),
        };
        let prog = aggregate_infer_program(base);
        for s in &prog.statements {
            if let Statement::VarDecl { name, type_ann, .. } = s {
                if name.as_ref() == "b" {
                    // No array factory / array-param fns ⇒ S-D candidacy fails ⇒ `b` unstamped.
                    assert!(type_ann.is_none(), "non-array struct local must stay unannotated");
                }
            }
        }
    }

    // ---- S-D: whole-program unboxing candidacy + stamping --------------------

    /// The full nbody factory + array-fn chain, trimmed to the structural essentials.
    const NBODY_SHAPE: &str = "\
        let SOLAR_MASS = 39.47841760435743\n\
        function body(x, y, z, vx, vy, vz, mass) {\n\
          return { x: x, y: y, z: z, vx: vx, vy: vy, vz: vz, mass: mass }\n\
        }\n\
        function makeBodies() {\n\
          let sun = body(0, 0, 0, 0, 0, 0, SOLAR_MASS)\n\
          let jup = body(1, 1, 1, 1, 1, 1, 1)\n\
          return [sun, jup]\n\
        }\n\
        function offsetMomentum(bodies) {\n\
          let px = 0\n\
          for (let i = 0; i < bodies.length; i++) { let b = bodies[i]; px = px + b.vx * b.mass }\n\
          bodies[0].vx = -px / SOLAR_MASS\n\
        }\n\
        function advance(bodies, dt) {\n\
          let n = bodies.length\n\
          for (let i = 0; i < n; i++) {\n\
            let bi = bodies[i]\n\
            for (let j = i + 1; j < n; j++) {\n\
              let bj = bodies[j]\n\
              let dx = bi.x - bj.x\n\
              bi.vx = bi.vx - dx\n\
              bj.vx = bj.vx + dx\n\
            }\n\
          }\n\
        }\n\
        function energy(bodies) {\n\
          let e = 0\n\
          let n = bodies.length\n\
          for (let i = 0; i < n; i++) { let bi = bodies[i]; e = e + bi.mass }\n\
          return e\n\
        }\n\
        let bodies = makeBodies()\n\
        offsetMomentum(bodies)\n\
        for (let s = 0; s < 10; s++) { advance(bodies, 0.01) }\n\
        let check = energy(bodies)\n\
        console.log(check)";

    #[test]
    fn sd_nbody_shape_is_unbox_candidate() {
        let a = analyze(NBODY_SHAPE);
        // Exactly one struct alias, and the whole group passes candidacy.
        assert_eq!(a.struct_decls.len(), 1);
        let alias = a.unbox_alias.clone();
        assert!(alias.is_some(), "nbody shape must be an unbox candidate");
        let alias = alias.unwrap();
        // body=Struct, makeBodies=ArrayOfStruct, bodies local = ArrayOfStruct.
        assert!(matches!(a.fn_return.get("body"), Some(AggReturn::Struct(s)) if *s == alias));
        assert!(matches!(a.fn_return.get("makeBodies"), Some(AggReturn::ArrayOfStruct(s)) if *s == alias));
        assert!(a.array_locals.contains(&"bodies".to_string()));
        // advance/energy/offsetMomentum all recognised as array-param fns at param 0.
        for f in ["advance", "energy", "offsetMomentum"] {
            let (pi, pn) = a.array_param_fns.get(f).unwrap_or_else(|| panic!("{f} not in group"));
            assert_eq!(*pi, 0);
            assert_eq!(pn, "bodies");
        }
    }

    #[test]
    fn sd_stamps_struct_and_array_annotations() {
        let parsed = parse(NBODY_SHAPE).unwrap();
        let base = Program {
            statements: infer_statements(&parsed.statements, &mut InferCtx::new()),
        };
        let prog = aggregate_infer_program(base);
        let alias = analyze_aggregate(&Program {
            statements: infer_statements(&parse(NBODY_SHAPE).unwrap().statements, &mut InferCtx::new()),
        })
        .unbox_alias
        .unwrap();
        let mut saw_body_ret = false;
        let mut saw_makebodies_ret = false;
        let mut saw_advance_param = false;
        let mut saw_bodies_local = false;
        for s in &prog.statements {
            match s {
                Statement::FunDecl { name, params, return_type, .. } => match name.as_ref() {
                    "body" => {
                        saw_body_ret = matches!(return_type, Some(TypeAnnotation::Simple(a, _)) if a.as_ref() == alias);
                    }
                    "makeBodies" => {
                        saw_makebodies_ret = matches!(return_type,
                            Some(TypeAnnotation::Array(b)) if matches!(b.as_ref(), TypeAnnotation::Simple(a, _) if a.as_ref() == alias));
                    }
                    "advance" => {
                        if let Some(FunParam::Simple(tp)) = params.first() {
                            saw_advance_param = matches!(&tp.type_ann,
                                Some(TypeAnnotation::Array(b)) if matches!(b.as_ref(), TypeAnnotation::Simple(a, _) if a.as_ref() == alias));
                        }
                    }
                    _ => {}
                },
                Statement::VarDecl { name, type_ann, .. } if name.as_ref() == "bodies" => {
                    saw_bodies_local = matches!(type_ann,
                        Some(TypeAnnotation::Array(b)) if matches!(b.as_ref(), TypeAnnotation::Simple(a, _) if a.as_ref() == alias));
                }
                _ => {}
            }
        }
        assert!(saw_body_ret, "body return must be stamped `: alias`");
        assert!(saw_makebodies_ret, "makeBodies return must be stamped `: alias[]`");
        assert!(saw_advance_param, "advance bodies param must be stamped `: alias[]`");
        assert!(saw_bodies_local, "top-level bodies local must be stamped `: alias[]`");
    }

    #[test]
    fn sd_bails_on_strict_eq_on_body() {
        // `===` on an element alias compares references in JS — not unboxable. Whole group bails.
        let src = "\
            function body(x) { return { x: x } }\n\
            function makeBodies() { let a = body(1); return [a] }\n\
            function check(bodies) { let b0 = bodies[0]; let b1 = bodies[0]; return b0 === b1 }\n\
            let bodies = makeBodies()\n\
            check(bodies)";
        let a = analyze(src);
        assert!(a.unbox_alias.is_none(), "=== on a body must bail the unbox group");
    }

    #[test]
    fn sd_bails_on_array_push_reshape() {
        // `bodies.push(...)` reshapes the array — not a fixed `Vec`. Bail.
        let src = "\
            function body(x) { return { x: x } }\n\
            function makeBodies() { let a = body(1); return [a] }\n\
            function grow(bodies) { bodies.push(body(2)) }\n\
            let bodies = makeBodies()\n\
            grow(bodies)";
        let a = analyze(src);
        assert!(a.unbox_alias.is_none(), "array reshape must bail the unbox group");
    }

    #[test]
    fn sd_bails_on_array_escape_to_console() {
        // Passing the whole array to console.log escapes it to the boxed world. Bail.
        let src = "\
            function body(x) { return { x: x } }\n\
            function makeBodies() { let a = body(1); return [a] }\n\
            function energy(bodies) { let b = bodies[0]; return b.x }\n\
            let bodies = makeBodies()\n\
            energy(bodies)\n\
            console.log(bodies)";
        let a = analyze(src);
        assert!(a.unbox_alias.is_none(), "array escape to console.log must bail");
    }

    #[test]
    fn sd_bails_on_string_field_struct() {
        // A string field ⇒ no struct factory at all ⇒ no unbox candidacy.
        let src = "\
            function body(x) { return { x: x, name: \"a\" } }\n\
            function makeBodies() { let a = body(1); return [a] }\n\
            function energy(bodies) { let b = bodies[0]; return b.x }\n\
            let bodies = makeBodies()\n\
            energy(bodies)";
        let a = analyze(src);
        assert!(a.unbox_alias.is_none(), "string field bails the struct factory → no unbox");
    }
}
