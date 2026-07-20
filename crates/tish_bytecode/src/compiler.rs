//! AST to bytecode compiler.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    ExportDeclaration, Expr, FunParam, JsxAttrValue, JsxChild, JsxProp, Literal, LogicalAssignOp,
    MemberProp, ObjectProp, Program, Span, Statement,
};

use crate::chunk::{Chunk, Constant};
use crate::encoding::{binop_to_u8, compound_op_to_u8, unaryop_to_u8};
use crate::opcode::{MathBinaryFn, MathUnaryFn, Opcode};

enum SimpleMapResult {
    Identity,
    BinOp(BinOp, Constant, bool), // op, constant, param_on_left
}

/// Provably evaluates to a `String` at runtime — the safety gate for the plain-assign `AppendLocal`
/// builder fast path (#186). A string literal, a template literal, or an `+` chain containing one
/// (JS `+` string-coerces the whole expression). Conservative: an unknown/numeric operand returns
/// `false`, so a numeric accumulator never routes through the builder.
fn is_string_typed(expr: &Expr) -> bool {
    match expr {
        Expr::Literal {
            value: Literal::String(_),
            ..
        } => true,
        Expr::TemplateLiteral { .. } => true,
        Expr::Binary {
            left,
            op: BinOp::Add,
            right,
            ..
        } => is_string_typed(left) || is_string_typed(right),
        _ => false,
    }
}

fn literal_to_constant(expr: &Expr) -> Option<Constant> {
    if let Expr::Literal { value, .. } = expr {
        Some(match value {
            Literal::Number(n) => Constant::Number(*n),
            Literal::String(s) => Constant::String(Arc::clone(s)),
            Literal::Bool(b) => Constant::Bool(*b),
            Literal::Null => Constant::Null,
        })
    } else {
        None
    }
}

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CompileError {}

/// Loop boundary for break/continue.
struct LoopInfo {
    break_patches: Vec<usize>,
    /// Operand positions for `continue`: either `JumpBack` (while / do-while / for-of) or `Jump`
    /// (C-style `for`, where the update clause is emitted after the body).
    continue_patches: Vec<usize>,
    /// When true, [`Opcode::Jump`] placeholders in `continue_patches` are patched forward with
    /// [`Self::patch_jump`]. When false, they are [`Opcode::JumpBack`] patched with
    /// [`Self::patch_jump_back`].
    continue_is_forward_jump: bool,
}

/// Switch boundary: break exits the switch.
struct SwitchInfo {
    break_patches: Vec<usize>,
}

/// Innermost break/continue target for unwinding `EnterBlock` before a jump.
#[derive(Clone, Copy)]
enum Breakable {
    /// `usize` = `block_depth` before the loop body (same as Continue unwind target).
    Loop { unwind_depth: usize },
    /// `usize` = `block_depth` before the switch statement.
    Switch { unwind_depth: usize },
}

struct Compiler<'a> {
    chunk: &'a mut Chunk,
    /// Current scope: variable name -> (depth, is_captured). Depth 0 = local.
    scope: Vec<HashMap<Arc<str>, bool>>,
    /// Stack of loop info for break/continue.
    loop_stack: Vec<LoopInfo>,
    switch_stack: Vec<SwitchInfo>,
    /// Parallel to nested loops/switches: innermost target for break/continue block unwind.
    breakable_stack: Vec<Breakable>,
    /// Nesting depth of emitted `EnterBlock` (lexical blocks) not yet closed on the compile path.
    block_depth: usize,
    /// When true (REPL mode), last ExprStmt leaves its value on the stack and we skip trailing LoadConst Null.
    retain_last_expr: bool,
    /// When `Some`, this chunk is being compiled in the SIMPLE slot mode: identifier references
    /// resolve to frame slots (`LoadLocal`) via this param→slot map instead of name-keyed `LoadVar`.
    /// Set only for self-contained param-only functions (see [`simple_fn_slots`]).
    slot_ctx: Option<HashMap<Arc<str>, u16>>,
    /// GENERAL slot mode (`TISH_VM_SLOTS`, capture-aware): a block-scoped stack of name→slot maps,
    /// innermost last. Allocated DURING compilation so block scoping + shadowing are correct (each
    /// `let` gets a fresh slot; resolution walks innermost-first; a block pops its frame). Empty unless
    /// [`general_slots`] is set. Captured names (in [`slot_captured`]) are never allocated here — they
    /// stay name-based in `local_scope` (which closures capture).
    slot_scopes: Vec<HashMap<Arc<str>, u16>>,
    /// Names referenced by a nested closure (over-approx) → must stay name-based even in slot mode.
    slot_captured: HashSet<Arc<str>>,
    /// Monotonic slot allocator for [`slot_scopes`] (never reclaimed; final value = frame size).
    next_slot: u16,
    /// True while compiling a chunk in general slot mode (see [`slot_scopes`]).
    general_slots: bool,
    /// Active `finally` bodies of enclosing `try`s in the CURRENT function (innermost last). A
    /// `return` that escapes these trys must run each one on the way out (the bytecode VM jumps
    /// straight to the function return otherwise). Reset per function — nested fns get a fresh
    /// `Compiler`. The exception-unwind path is handled separately in the `Try` emitter.
    finally_stack: Vec<Statement>,
    /// When `Some(name)`, this chunk is the body of `fn name(...)` and `name`'s binding is provably
    /// stable (no param shadows it, no reassignment/redeclaration in the body — see [`stmt_rebinds`]).
    /// A direct call `name(args)` then compiles to `SelfCall` (no name lookup / closure dispatch; the
    /// JIT lowers it to a native recursive call). `None` for anonymous fns, top-level, or anywhere the
    /// self-binding can't be proven stable.
    self_fn_name: Option<Arc<str>>,
    /// #186 — `Math` is provably the global builtin: it is never rebound/shadowed anywhere in the
    /// program (via the conservative [`stmt_rebinds`] scan). Only then may `Math.<fn>(arg)` lower to
    /// the [`Opcode::MathUnary`] intrinsic, which the numeric JIT can compile without a shape guard.
    /// `false` on nested-fn compilers and whenever the scan can't prove stability.
    math_is_global: bool,
    /// #187: top-level function names provably stable across the whole program (see
    /// [`compute_stable_globals`]). A top-level `function N` in this set gets its chunk stamped with
    /// `global_name = Some(N)`, which lets the numeric JIT register it as a directly-callable callee.
    /// Threaded unchanged into every nested compiler (shared `Arc`).
    stable_globals: Arc<std::collections::HashSet<Arc<str>>>,
}

/// Does `e` reference only the given params (no free/global vars, no nested
/// functions, no mutation)? Such a function can run on a bare slot frame.
fn expr_is_param_only(e: &Expr, params: &HashSet<&str>) -> bool {
    match e {
        Expr::Literal { .. } => true,
        Expr::Ident { name, .. } => params.contains(name.as_ref()),
        Expr::Binary { left, right, .. } => {
            expr_is_param_only(left, params) && expr_is_param_only(right, params)
        }
        Expr::Unary { operand, .. } => expr_is_param_only(operand, params),
        Expr::Call { callee, args, .. } => {
            expr_is_param_only(callee, params)
                && args.iter().all(|a| match a {
                    CallArg::Expr(x) => expr_is_param_only(x, params),
                    CallArg::Spread(_) => false,
                })
        }
        Expr::Member { object, prop, .. } => {
            expr_is_param_only(object, params)
                && match prop {
                    MemberProp::Name { .. } => true,
                    MemberProp::Expr(x) => expr_is_param_only(x, params),
                }
        }
        Expr::Index { object, index, .. } => {
            expr_is_param_only(object, params) && expr_is_param_only(index, params)
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_is_param_only(cond, params)
                && expr_is_param_only(then_branch, params)
                && expr_is_param_only(else_branch, params)
        }
        Expr::NullishCoalesce { left, right, .. } => {
            expr_is_param_only(left, params) && expr_is_param_only(right, params)
        }
        Expr::Array { elements, .. } => elements.iter().all(|el| match el {
            ArrayElement::Expr(x) => expr_is_param_only(x, params),
            ArrayElement::Spread(_) => false,
        }),
        Expr::Object { props, .. } => props.iter().all(|p| match p {
            ObjectProp::KeyValue(_, x, _) => expr_is_param_only(x, params),
            ObjectProp::Spread(_) => false,
        }),
        Expr::TemplateLiteral { exprs, .. } => exprs.iter().all(|x| expr_is_param_only(x, params)),
        Expr::TypeOf { operand, .. } => expr_is_param_only(operand, params),
        // Mutation, nested fns, async, jsx, native, `new` — not eligible.
        _ => false,
    }
}

/// Statement form of [`expr_is_param_only`]. Only the small set of statements a
/// pure leaf function body uses is allowed; anything that declares a binding or
/// loops bails (those keep the name-based path).
fn stmt_is_param_only(s: &Statement, params: &HashSet<&str>) -> bool {
    match s {
        Statement::Block { statements, .. } => {
            statements.iter().all(|st| stmt_is_param_only(st, params))
        }
        Statement::Multi { statements, .. } => {
            statements.iter().all(|st| stmt_is_param_only(st, params))
        }
        Statement::ExprStmt { expr, .. } => expr_is_param_only(expr, params),
        Statement::Return { value, .. } => {
            value.as_ref().is_none_or(|e| expr_is_param_only(e, params))
        }
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_is_param_only(cond, params)
                && stmt_is_param_only(then_branch, params)
                && else_branch
                    .as_ref()
                    .is_none_or(|b| stmt_is_param_only(b, params))
        }
        _ => false,
    }
}

/// If a function with these `params` (all simple, no rest) has a body that
/// references only its params, returns the param→slot map for slot-based
/// compilation. Slots are the parameter positions (0-based). Returns `None`
/// when the function must use the name-based path (captures outer scope,
/// declares locals, mutates, or defines nested functions).
fn simple_fn_slots(
    params: &[FunParam],
    has_rest: bool,
    body_ok: impl FnOnce(&HashSet<&str>) -> bool,
) -> Option<HashMap<Arc<str>, u16>> {
    if has_rest {
        return None;
    }
    let mut map: HashMap<Arc<str>, u16> = HashMap::with_capacity(params.len());
    for (i, p) in params.iter().enumerate() {
        match p {
            FunParam::Simple(tp) => {
                map.insert(Arc::clone(&tp.name), i as u16);
            }
            FunParam::Destructure { .. } => return None,
        }
    }
    let pset: HashSet<&str> = map.keys().map(|k| k.as_ref()).collect();
    if body_ok(&pset) {
        Some(map)
    } else {
        None
    }
}

/// Capture-aware general slot-based locals (params + uncaptured body/top-level `let`s → frame slots).
/// **Default ON** — validated across the full cross-backend suite + the compute micros (−22..27%) +
/// the `main.tish` bundle (−22%). Set `TISH_VM_SLOTS=0` to disable (name-based, the old path).
fn slots_enabled() -> bool {
    std::env::var("TISH_VM_SLOTS")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Is `name` bound by one of `params` (so it would shadow a function's own name)? Conservative:
/// any destructuring param returns `true` (it could bind `name` via a nested pattern we don't analyze).
fn params_bind_name(params: &[FunParam], name: &str) -> bool {
    params.iter().any(|p| match p {
        FunParam::Simple(tp) => tp.name.as_ref() == name,
        FunParam::Destructure { .. } => true,
    })
}

/// #187: does any parameter DEFAULT expression rebind `name`? A default like `(y = (foo = evil))`
/// reassigns `foo` when the function is called, so a callee reached through it is NOT stable. The
/// stability walk must scan these (they are otherwise invisible to the body scan).
fn params_default_rebinds(params: &[FunParam], name: &str) -> bool {
    params.iter().any(|p| {
        let default = match p {
            FunParam::Simple(tp) => &tp.default,
            FunParam::Destructure { default, .. } => default,
        };
        default.as_ref().is_some_and(|e| expr_rebinds(e, name))
    })
}

/// Conservative scan: does `name` get REBOUND (assigned `=`, `+=`, `??=`, `++`/`--`, or re-declared
/// via `let`/`for-of`) anywhere in `s`? Returns `true` on a rebind OR on any node it can't fully
/// analyze. Used to decide whether `fn NAME`'s body may emit `SelfCall` for `NAME(...)`: only when
/// NAME's binding is PROVABLY stable throughout the body, because a wrong `SelfCall` would call the
/// original chunk after a reassignment — a silent miscompile. Erring toward `true` only costs the
/// optimization, never correctness.
fn stmt_rebinds(s: &Statement, name: &str) -> bool {
    match s {
        Statement::Block { statements, .. } => statements.iter().any(|s| stmt_rebinds(s, name)),
        Statement::Multi { statements, .. } => statements.iter().any(|s| stmt_rebinds(s, name)),
        Statement::VarDecl { name: n, init, .. } => {
            n.as_ref() == name || init.as_ref().is_some_and(|e| expr_rebinds(e, name))
        }
        Statement::ExprStmt { expr, .. } => expr_rebinds(expr, name),
        Statement::Return { value, .. } => value.as_ref().is_some_and(|e| expr_rebinds(e, name)),
        Statement::Throw { value, .. } => expr_rebinds(value, name),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_rebinds(cond, name)
                || stmt_rebinds(then_branch, name)
                || else_branch.as_ref().is_some_and(|s| stmt_rebinds(s, name))
        }
        Statement::While { cond, body, .. } => expr_rebinds(cond, name) || stmt_rebinds(body, name),
        Statement::DoWhile { body, cond, .. } => {
            stmt_rebinds(body, name) || expr_rebinds(cond, name)
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(|s| stmt_rebinds(s, name))
                || cond.as_ref().is_some_and(|e| expr_rebinds(e, name))
                || update.as_ref().is_some_and(|e| expr_rebinds(e, name))
                || stmt_rebinds(body, name)
        }
        Statement::ForOf {
            name: n,
            iterable: e,
            body,
            ..
        }
        | Statement::ForIn {
            name: n,
            object: e,
            body,
            ..
        } => n.as_ref() == name || expr_rebinds(e, name) || stmt_rebinds(body, name),
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            expr_rebinds(expr, name)
                || cases.iter().any(|(t, body)| {
                    t.as_ref().is_some_and(|e| expr_rebinds(e, name))
                        || body.iter().any(|s| stmt_rebinds(s, name))
                })
                || default_body
                    .as_ref()
                    .is_some_and(|b| b.iter().any(|s| stmt_rebinds(s, name)))
        }
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            stmt_rebinds(body, name)
                || catch_body.as_ref().is_some_and(|s| stmt_rebinds(s, name))
                || finally_body.as_ref().is_some_and(|s| stmt_rebinds(s, name))
        }
        Statement::Break { .. } | Statement::Continue { .. } => false,
        // VarDeclDestructure (could bind `name`), FunDecl (could shadow), and any unknown construct
        // → conservative: assume it may rebind `name`.
        _ => true,
    }
}

/// Expression half of [`stmt_rebinds`]. `true` if `name` is an assignment/update target, or unknown.
fn expr_rebinds(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Assign { name: n, value, .. }
        | Expr::CompoundAssign { name: n, value, .. }
        | Expr::LogicalAssign { name: n, value, .. } => {
            n.as_ref() == name || expr_rebinds(value, name)
        }
        Expr::PostfixInc { name: n, .. }
        | Expr::PostfixDec { name: n, .. }
        | Expr::PrefixInc { name: n, .. }
        | Expr::PrefixDec { name: n, .. } => n.as_ref() == name,
        Expr::Literal { .. } | Expr::Ident { .. } => false,
        Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
            expr_rebinds(left, name) || expr_rebinds(right, name)
        }
        Expr::Unary { operand, .. }
        | Expr::TypeOf { operand, .. }
        | Expr::Await { operand, .. } => expr_rebinds(operand, name),
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_rebinds(cond, name)
                || expr_rebinds(then_branch, name)
                || expr_rebinds(else_branch, name)
        }
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            expr_rebinds(callee, name)
                || args.iter().any(|a| match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => expr_rebinds(e, name),
                })
        }
        Expr::Member { object, .. } => expr_rebinds(object, name),
        Expr::Index { object, index, .. } => {
            expr_rebinds(object, name) || expr_rebinds(index, name)
        }
        Expr::Array { elements, .. } => elements.iter().any(|el| match el {
            ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_rebinds(e, name),
        }),
        Expr::Object { props, .. } => props.iter().any(|p| match p {
            ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => expr_rebinds(e, name),
        }),
        Expr::MemberAssign { object, value, .. } => {
            expr_rebinds(object, name) || expr_rebinds(value, name)
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => expr_rebinds(object, name) || expr_rebinds(index, name) || expr_rebinds(value, name),
        Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(|e| expr_rebinds(e, name)),
        // A nested closure could reassign the outer `name` — in its body OR a param default (which
        // evaluates in the enclosing scope). Recurse into both (over-conservative if it shadows, which
        // only costs the optimization). #187: the param-default scan closes the `(a = (foo = x)) => …` hole.
        Expr::ArrowFunction { params, body, .. } => {
            params_default_rebinds(params, name)
                || match body {
                    ArrowBody::Expr(e) => expr_rebinds(e, name),
                    ArrowBody::Block(s) => stmt_rebinds(s, name),
                }
        }
        // Jsx, NativeModuleLoad, and anything unknown → conservative.
        _ => true,
    }
}

/// #187: whole-program scan — does `name` get REBOUND (assigned/updated, redeclared via `let`/`const`/
/// `for-of`/`catch`, shadowed by a param, or (re)declared via a `function name`) anywhere in `s`,
/// INCLUDING inside every function body? Unlike [`stmt_rebinds`] (which returns `true` on any `FunDecl`
/// so it can't run program-wide), this has an explicit `FunDecl` arm: a `function name` is itself a
/// binding, and every function body is recursed into. `true` (or any node it can't analyze) ⇒ NOT
/// stable — which only forgoes the cross-function-call optimization, never risks a miscompile.
fn name_rebinds_in_stmt(s: &Statement, name: &str) -> bool {
    match s {
        Statement::FunDecl {
            name: n,
            params,
            rest_param,
            body,
            ..
        } => {
            n.as_ref() == name
                || params_bind_name(params, name)
                || params_default_rebinds(params, name)
                || rest_param
                    .as_ref()
                    .is_some_and(|rp| rp.name.as_ref() == name)
                || name_rebinds_in_stmt(body, name)
        }
        Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
            statements.iter().any(|s| name_rebinds_in_stmt(s, name))
        }
        Statement::VarDecl { name: n, init, .. } => {
            n.as_ref() == name || init.as_ref().is_some_and(|e| expr_rebinds(e, name))
        }
        Statement::ExprStmt { expr, .. } => expr_rebinds(expr, name),
        Statement::Return { value, .. } => value.as_ref().is_some_and(|e| expr_rebinds(e, name)),
        Statement::Throw { value, .. } => expr_rebinds(value, name),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_rebinds(cond, name)
                || name_rebinds_in_stmt(then_branch, name)
                || else_branch
                    .as_ref()
                    .is_some_and(|s| name_rebinds_in_stmt(s, name))
        }
        Statement::While { cond, body, .. } => {
            expr_rebinds(cond, name) || name_rebinds_in_stmt(body, name)
        }
        Statement::DoWhile { body, cond, .. } => {
            name_rebinds_in_stmt(body, name) || expr_rebinds(cond, name)
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(|s| name_rebinds_in_stmt(s, name))
                || cond.as_ref().is_some_and(|e| expr_rebinds(e, name))
                || update.as_ref().is_some_and(|e| expr_rebinds(e, name))
                || name_rebinds_in_stmt(body, name)
        }
        Statement::ForOf {
            name: n,
            iterable: e,
            body,
            ..
        }
        | Statement::ForIn {
            name: n,
            object: e,
            body,
            ..
        } => n.as_ref() == name || expr_rebinds(e, name) || name_rebinds_in_stmt(body, name),
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            expr_rebinds(expr, name)
                || cases.iter().any(|(t, body)| {
                    t.as_ref().is_some_and(|e| expr_rebinds(e, name))
                        || body.iter().any(|s| name_rebinds_in_stmt(s, name))
                })
                || default_body
                    .as_ref()
                    .is_some_and(|b| b.iter().any(|s| name_rebinds_in_stmt(s, name)))
        }
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            name_rebinds_in_stmt(body, name)
                || catch_body
                    .as_ref()
                    .is_some_and(|s| name_rebinds_in_stmt(s, name))
                || finally_body
                    .as_ref()
                    .is_some_and(|s| name_rebinds_in_stmt(s, name))
        }
        Statement::Break { .. } | Statement::Continue { .. } => false,
        // VarDeclDestructure (could bind `name`) + any unknown construct → conservative.
        _ => true,
    }
}

/// #187: the set of top-level function names that are PROVABLY stable across the whole program — each
/// declared exactly once as a top-level `function N`, never also bound by a top-level `let`/`var`/
/// `const`, and never reassigned/redeclared/shadowed anywhere (via [`name_rebinds_in_stmt`], skipping
/// the single defining declaration but still recursing into its body). A direct native call to such a
/// callee can never dispatch a stale function, because the binding can never change.
fn compute_stable_globals(program: &Program) -> std::collections::HashSet<Arc<str>> {
    let mut fn_count: HashMap<Arc<str>, usize> = HashMap::new();
    let mut nonfn_toplevel: std::collections::HashSet<Arc<str>> = std::collections::HashSet::new();
    for s in &program.statements {
        match s {
            Statement::FunDecl { name, .. } => *fn_count.entry(Arc::clone(name)).or_insert(0) += 1,
            Statement::VarDecl { name, .. } => {
                nonfn_toplevel.insert(Arc::clone(name));
            }
            _ => {}
        }
    }
    let mut stable: std::collections::HashSet<Arc<str>> = std::collections::HashSet::new();
    'cand: for (name, &count) in &fn_count {
        if count != 1 || nonfn_toplevel.contains(name) {
            continue;
        }
        for s in &program.statements {
            match s {
                // The single defining `function name`: not a rebind, but its body/params still count.
                Statement::FunDecl {
                    name: n,
                    params,
                    rest_param,
                    body,
                    ..
                } if n == name => {
                    if params_bind_name(params, name)
                        || params_default_rebinds(params, name)
                        || rest_param
                            .as_ref()
                            .is_some_and(|rp| rp.name.as_ref() == name.as_ref())
                        || name_rebinds_in_stmt(body, name)
                    {
                        continue 'cand;
                    }
                }
                other => {
                    if name_rebinds_in_stmt(other, name) {
                        continue 'cand;
                    }
                }
            }
        }
        stable.insert(Arc::clone(name));
    }
    stable
}

/// One conservative pass computing the over-approximated CAPTURED set: every identifier that appears
/// textually inside any nested closure (`ArrowFunction`/`FunDecl`) — its body AND its parameter
/// defaults (which evaluate in the enclosing scope, e.g. `(a = secret) => a` captures `secret`). A
/// captured local must stay name-based in `local_scope` (which closures capture); only uncaptured
/// locals are slotted. Recurses ALL ordinary control flow (so it finds every closure → the capture
/// set is complete); returns `false` ONLY on ambient/module constructs it cannot traverse, so the
/// caller leaves the whole chunk name-based (safe default-bail: a missed closure could otherwise let a
/// captured local be wrongly slotted). Slot ALLOCATION happens during compilation (scope-aware), not
/// here — so block scoping + shadowing are handled by the slot-scope stack, not a flat map.
#[derive(Default)]
struct SlotScan {
    captured: HashSet<Arc<str>>,
}

impl SlotScan {
    fn stmt(&mut self, s: &Statement, in_closure: bool) -> bool {
        match s {
            Statement::Block { statements, .. } => {
                statements.iter().all(|s| self.stmt(s, in_closure))
            }
            Statement::Multi { statements, .. } => {
                statements.iter().all(|s| self.stmt(s, in_closure))
            }
            Statement::VarDecl { init, .. } => {
                init.as_ref().is_none_or(|e| self.expr(e, in_closure))
            }
            Statement::VarDeclDestructure { init, .. } => self.expr(init, in_closure),
            Statement::ExprStmt { expr, .. } => self.expr(expr, in_closure),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(cond, in_closure)
                    && self.stmt(then_branch, in_closure)
                    && else_branch
                        .as_ref()
                        .is_none_or(|s| self.stmt(s, in_closure))
            }
            Statement::While { cond, body, .. } => {
                self.expr(cond, in_closure) && self.stmt(body, in_closure)
            }
            Statement::DoWhile { body, cond, .. } => {
                self.stmt(body, in_closure) && self.expr(cond, in_closure)
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                init.as_ref().is_none_or(|i| self.stmt(i, in_closure))
                    && cond.as_ref().is_none_or(|e| self.expr(e, in_closure))
                    && update.as_ref().is_none_or(|e| self.expr(e, in_closure))
                    && self.stmt(body, in_closure)
            }
            Statement::ForOf { iterable, body, .. } => {
                self.expr(iterable, in_closure) && self.stmt(body, in_closure)
            }
            Statement::Return { value, .. } => {
                value.as_ref().is_none_or(|e| self.expr(e, in_closure))
            }
            Statement::Throw { value, .. } => self.expr(value, in_closure),
            Statement::Break { .. } | Statement::Continue { .. } => true,
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                if !self.expr(expr, in_closure) {
                    return false;
                }
                for (test, body) in cases {
                    if let Some(t) = test {
                        if !self.expr(t, in_closure) {
                            return false;
                        }
                    }
                    if !body.iter().all(|s| self.stmt(s, in_closure)) {
                        return false;
                    }
                }
                default_body
                    .as_ref()
                    .is_none_or(|b| b.iter().all(|s| self.stmt(s, in_closure)))
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                self.stmt(body, in_closure)
                    && catch_body.as_ref().is_none_or(|s| self.stmt(s, in_closure))
                    && finally_body
                        .as_ref()
                        .is_none_or(|s| self.stmt(s, in_closure))
            }
            // A nested named function: its param defaults (enclosing-scope) + whole body capture.
            Statement::FunDecl { params, body, .. } => {
                self.scan_closure_param_defaults(params) && self.stmt(body, true)
            }
            // Ambient/module constructs (Import/Export/TypeAlias/DeclareVar/DeclareFun) → bail.
            _ => false,
        }
    }

    fn expr(&mut self, e: &Expr, in_closure: bool) -> bool {
        match e {
            Expr::Literal { .. } => true,
            Expr::Ident { name, .. } => {
                if in_closure {
                    self.captured.insert(Arc::clone(name));
                }
                true
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left, in_closure) && self.expr(right, in_closure)
            }
            Expr::Unary { operand, .. }
            | Expr::TypeOf { operand, .. }
            | Expr::Await { operand, .. } => self.expr(operand, in_closure),
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(cond, in_closure)
                    && self.expr(then_branch, in_closure)
                    && self.expr(else_branch, in_closure)
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.expr(left, in_closure) && self.expr(right, in_closure)
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                self.expr(callee, in_closure)
                    && args.iter().all(|a| match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => self.expr(e, in_closure),
                    })
            }
            Expr::Member { object, .. } => self.expr(object, in_closure),
            Expr::Index { object, index, .. } => {
                self.expr(object, in_closure) && self.expr(index, in_closure)
            }
            Expr::Array { elements, .. } => elements.iter().all(|el| match el {
                ArrayElement::Expr(e) | ArrayElement::Spread(e) => self.expr(e, in_closure),
            }),
            Expr::Object { props, .. } => props.iter().all(|p| match p {
                ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => self.expr(e, in_closure),
            }),
            Expr::Assign { name, value, .. }
            | Expr::CompoundAssign { name, value, .. }
            | Expr::LogicalAssign { name, value, .. } => {
                if in_closure {
                    self.captured.insert(Arc::clone(name));
                }
                self.expr(value, in_closure)
            }
            Expr::MemberAssign { object, value, .. } => {
                self.expr(object, in_closure) && self.expr(value, in_closure)
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                self.expr(object, in_closure)
                    && self.expr(index, in_closure)
                    && self.expr(value, in_closure)
            }
            Expr::PostfixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PrefixDec { name, .. } => {
                if in_closure {
                    self.captured.insert(Arc::clone(name));
                }
                true
            }
            Expr::TemplateLiteral { exprs, .. } => exprs.iter().all(|e| self.expr(e, in_closure)),
            Expr::ArrowFunction { params, body, .. } => {
                if !self.scan_closure_param_defaults(params) {
                    return false;
                }
                match body {
                    ArrowBody::Expr(e) => self.expr(e, true),
                    ArrowBody::Block(s) => self.stmt(s, true),
                }
            }
            // Jsx, NativeModuleLoad → bail.
            _ => false,
        }
    }

    /// A nested closure's parameter default expressions evaluate in the ENCLOSING scope → captured.
    fn scan_closure_param_defaults(&mut self, params: &[FunParam]) -> bool {
        for p in params {
            let default = match p {
                FunParam::Simple(tp) => &tp.default,
                FunParam::Destructure { default, .. } => default,
            };
            if let Some(d) = default {
                if !self.expr(d, true) {
                    return false;
                }
            }
        }
        true
    }
}

/// Capture-aware eligibility for general slot-based locals in a FUNCTION. Returns the captured-name set
/// (names that must stay name-based) when eligible, else `None` (compile name-based). Eligible iff the
/// flag is on, no rest param, all params simple, the body fully analysable, and no PARAM is captured
/// (the VM binds params into slots 0..n, but a closure reads captures by name from `local_scope`).
fn slot_analyze(
    params: &[FunParam],
    has_rest: bool,
    body: &Statement,
) -> Option<HashSet<Arc<str>>> {
    if !slots_enabled() || has_rest {
        return None;
    }
    for p in params {
        if let FunParam::Destructure { .. } = p {
            return None;
        }
    }
    let mut scan = SlotScan::default();
    if !scan.stmt(body, false) {
        return None;
    }
    for p in params {
        if let FunParam::Simple(tp) = p {
            if scan.captured.contains(&tp.name) {
                return None;
            }
        }
    }
    Some(scan.captured)
}

/// Same, for the TOP-LEVEL program (no params). Caller must additionally ensure non-REPL mode.
fn slot_analyze_toplevel(statements: &[Statement]) -> Option<HashSet<Arc<str>>> {
    if !slots_enabled() {
        return None;
    }
    let mut scan = SlotScan::default();
    for s in statements {
        if !scan.stmt(s, false) {
            return None;
        }
    }
    Some(scan.captured)
}

impl<'a> Compiler<'a> {
    /// Resolve a name to its frame slot. `None` ⇒ name-based (a captured local, a global, or a
    /// builtin) — the single source of truth for slot-vs-name. Checks the simple param-only map first
    /// (a chunk is in exactly one mode), then the general scope stack innermost-first (shadowing).
    #[inline]
    fn resolve_slot(&self, name: &str) -> Option<u16> {
        if let Some(m) = self.slot_ctx.as_ref() {
            if let Some(s) = m.get(name) {
                return Some(*s);
            }
        }
        self.slot_scopes
            .iter()
            .rev()
            .find_map(|m| m.get(name).copied())
    }

    /// Emit a variable READ: `LoadLocal` if slotted, else name-based `LoadVar`.
    fn emit_var_load(&mut self, name: &Arc<str>) {
        if let Some(slot) = self.resolve_slot(name) {
            self.emit_u16(Opcode::LoadLocal, slot);
        } else {
            let idx = self.name_idx(name);
            self.emit_u16(Opcode::LoadVar, idx);
        }
    }

    /// Emit a variable WRITE (value already on stack): `StoreLocal` if slotted, else `StoreVar`.
    fn emit_var_store(&mut self, name: &Arc<str>) {
        if let Some(slot) = self.resolve_slot(name) {
            self.emit_u16(Opcode::StoreLocal, slot);
        } else {
            let idx = self.name_idx(name);
            self.emit_u16(Opcode::StoreVar, idx);
        }
    }

    fn new(chunk: &'a mut Chunk, retain_last_expr: bool) -> Self {
        Self {
            chunk,
            scope: vec![HashMap::new()],
            loop_stack: Vec::new(),
            switch_stack: Vec::new(),
            breakable_stack: Vec::new(),
            block_depth: 0,
            retain_last_expr,
            slot_ctx: None,
            slot_scopes: Vec::new(),
            slot_captured: HashSet::new(),
            next_slot: 0,
            general_slots: false,
            finally_stack: Vec::new(),
            self_fn_name: None,
            math_is_global: false,
            stable_globals: Arc::new(std::collections::HashSet::new()),
        }
    }

    /// Begin a lexical block: push a name-scope frame and (in general slot mode) a slot-scope frame,
    /// so block-local `let`s shadow correctly and are reclaimed at block end. Pair with [`exit_block_scope`].
    fn enter_block_scope(&mut self) {
        self.scope.push(HashMap::default());
        if self.general_slots {
            self.slot_scopes.push(HashMap::default());
        }
    }

    fn exit_block_scope(&mut self) {
        let _popped = self.scope.pop();
        if self.general_slots {
            self.slot_scopes.pop();
        }
    }

    /// Allocate a fresh frame slot for `name` in the innermost slot scope (general mode).
    fn declare_slot(&mut self, name: &Arc<str>) -> u16 {
        let slot = self.next_slot;
        self.next_slot += 1;
        if let Some(frame) = self.slot_scopes.last_mut() {
            frame.insert(Arc::clone(name), slot);
        }
        slot
    }

    /// Emit the pending `finally` bodies (innermost first) before a `return` escapes them. While
    /// emitting, the stack is cleared so a `return` *inside* one of these finallys doesn't recurse.
    fn emit_pending_finallys(&mut self) -> Result<(), CompileError> {
        if self.finally_stack.is_empty() {
            return Ok(());
        }
        let saved = std::mem::take(&mut self.finally_stack);
        for finally in saved.iter().rev() {
            self.compile_statement(finally)?;
        }
        self.finally_stack = saved;
        Ok(())
    }

    fn emit_exit_blocks_until_depth(&mut self, target_depth: usize) {
        let n = self.block_depth.saturating_sub(target_depth);
        for _ in 0..n {
            self.emit(Opcode::ExitBlock);
        }
    }

    /// C-style `for` init: bindings are not inside the `{ ... }` body for block-undo purposes.
    /// Formal parameters as VM slot names plus optional destructure patterns (one per formal).
    #[allow(clippy::type_complexity)] // (slot names, optional destructure patterns) — single-use return
    fn plan_function_params(
        params: &[FunParam],
    ) -> Result<(Vec<Arc<str>>, Vec<Option<DestructPattern>>), CompileError> {
        let mut names = Vec::with_capacity(params.len());
        let mut slots: Vec<Option<DestructPattern>> = Vec::with_capacity(params.len());
        let mut syn_counter = 0u32;
        for p in params {
            match p {
                FunParam::Simple(tp) => {
                    names.push(Arc::clone(&tp.name));
                    slots.push(None);
                }
                FunParam::Destructure {
                    pattern, default, ..
                } => {
                    if default.is_some() {
                        return Err(CompileError {
                            message: "Default values on destructuring parameters are not supported in bytecode"
                                .to_string(),
                        });
                    }
                    names.push(Arc::from(format!("__param_{}", syn_counter)));
                    syn_counter += 1;
                    slots.push(Some(pattern.clone()));
                }
            }
        }
        Ok((names, slots))
    }

    /// After VM binds positional args to `param_names`, load each destructure slot and bind pattern locals.
    fn emit_param_destructure_prologue(
        &mut self,
        param_names: &[Arc<str>],
        slots: &[Option<DestructPattern>],
    ) -> Result<(), CompileError> {
        debug_assert_eq!(param_names.len(), slots.len());
        for (name, slot) in param_names.iter().zip(slots.iter()) {
            if let Some(pattern) = slot {
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::LoadVar, idx);
                self.compile_destructure(pattern, false, false)?;
            }
        }
        Ok(())
    }

    /// Emit the default-parameter prologue: for each simple param `p_i` with a default,
    /// `if (arg i was not supplied) p_i = <default>`. Runs at the top of the function body so
    /// later defaults can reference earlier (already-bound) params, e.g. `(a, b = a + 1)`.
    ///
    /// Uses `ArgMissing(i)` (true iff `i >= argc`) + `JumpIfFalse` so the default applies only
    /// to *missing* positional args — matching the interpreter, where an explicit `null` keeps
    /// the `null` (tish has no `undefined`). The store mirrors variable resolution: a slot-based
    /// chunk writes the slot directly (`StoreLocal`); a name-based chunk binds the name
    /// (`DeclareVarPlain`, since a missing param is absent from the frame scope).
    fn emit_param_defaults_prologue(&mut self, params: &[FunParam]) -> Result<(), CompileError> {
        for (i, p) in params.iter().enumerate() {
            let FunParam::Simple(tp) = p else { continue };
            let Some(default_expr) = &tp.default else {
                continue;
            };
            self.emit_u16(Opcode::ArgMissing, i as u16);
            let skip = self.emit_jump(Opcode::JumpIfFalse);
            self.compile_expr(default_expr)?;
            let slot = self
                .slot_ctx
                .as_ref()
                .and_then(|m| m.get(tp.name.as_ref()))
                .copied();
            match slot {
                Some(slot) => self.emit_u16(Opcode::StoreLocal, slot),
                None => {
                    let idx = self.name_idx(&tp.name);
                    self.emit_u16(Opcode::DeclareVarPlain, idx);
                }
            }
            self.patch_jump(skip, self.chunk.code.len());
        }
        Ok(())
    }

    /// Names `let`/`const`-declared DIRECTLY in a loop body block (not nested blocks). Each is a
    /// fresh per-iteration binding (ES `let`), so closures created in the body must capture this
    /// iteration's value — registered via `LoopVarsBegin`.
    fn loop_body_block_lets(body: &Statement) -> Vec<Arc<str>> {
        let mut out = Vec::new();
        if let Statement::Block { statements, .. } = body {
            for s in statements {
                if let Statement::VarDecl { name, .. } = s {
                    out.push(Arc::clone(name));
                }
            }
        }
        out
    }

    fn compile_for_init_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::VarDecl {
                name,
                init,
                mutable: _,
                ..
            } => {
                if let Some(expr) = init {
                    self.compile_expr(expr)?;
                } else {
                    let idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                if self.general_slots && !self.slot_captured.contains(name.as_ref()) {
                    let slot = self.declare_slot(name);
                    self.emit_u16(Opcode::StoreLocal, slot);
                } else {
                    let idx = self.name_idx(name);
                    self.emit_u16(Opcode::DeclareVarPlain, idx);
                    self.scope
                        .last_mut()
                        .unwrap()
                        .insert(Arc::clone(name), false);
                }
            }
            Statement::VarDeclDestructure { pattern, init, .. } => {
                self.compile_expr(init)?;
                self.compile_destructure(pattern, false, true)?;
            }
            _ => self.compile_statement(stmt)?,
        }
        Ok(())
    }

    fn name_idx(&mut self, name: &Arc<str>) -> u16 {
        self.chunk.add_name(Arc::clone(name))
    }

    fn constant_idx(&mut self, c: Constant) -> u16 {
        self.chunk.add_constant(c)
    }

    fn emit(&mut self, op: Opcode) {
        self.chunk.write_u8(op as u8);
    }

    /// Record the source line of the code about to be emitted, for runtime error locations
    /// (issue #74). Cheap and deduped: only a line *change* adds a table entry.
    fn mark_line(&mut self, span: tishlang_ast::Span) {
        let offset = self.chunk.code.len();
        self.chunk.mark_line(offset, span.start.0 as u32);
    }

    fn emit_u8(&mut self, op: Opcode, v: u8) {
        self.chunk.write_u8(op as u8);
        self.chunk.write_u16(v as u16);
    }

    fn emit_u16(&mut self, op: Opcode, v: u16) {
        self.chunk.write_u8(op as u8);
        self.chunk.write_u16(v);
    }

    fn emit_jump(&mut self, op: Opcode) -> usize {
        let pos = self.chunk.code.len();
        self.chunk.write_u8(op as u8);
        self.chunk.write_u16(0); // placeholder
        pos + 1
    }

    /// Emit JumpBack with placeholder distance; patch later with patch_jump_back.
    fn emit_jump_back(&mut self) -> usize {
        let pos = self.chunk.code.len();
        self.chunk.write_u8(Opcode::JumpBack as u8);
        self.chunk.write_u16(0);
        pos + 1
    }

    fn patch_jump(&mut self, patch_pos: usize, target: usize) {
        let base = patch_pos + 2;
        let jump_offset = (target as i32).wrapping_sub(base as i32);
        let bytes = (jump_offset as i16).to_be_bytes();
        self.chunk.code[patch_pos] = bytes[0];
        self.chunk.code[patch_pos + 1] = bytes[1];
    }

    /// Compile `cond` in CONDITION position (if / while / for / do-while). The operand stack is EMPTY
    /// at every emitted `JumpIfFalse` — `&&` / `||` lower to pure control flow rather than the
    /// value-producing `Dup` + `BinOp` form, so a hot numeric loop whose only disqualifier was the
    /// condition shape stays eligible for the cranelift JIT (#167). Returns the `JumpIfFalse` patch
    /// sites the caller must point at its "condition is false" target. The condition value itself is
    /// never observed here, so truthiness-only lowering is exact; short-circuit order is preserved.
    fn compile_condition_jump_if_false(&mut self, cond: &Expr) -> Result<Vec<usize>, CompileError> {
        match cond {
            // `a && b` is false if EITHER operand is false — test each in order; both exit to the
            // same false-target, so concatenate their patch sites.
            Expr::Binary {
                left,
                op: BinOp::And,
                right,
                ..
            } => {
                let mut patches = self.compile_condition_jump_if_false(left)?;
                patches.extend(self.compile_condition_jump_if_false(right)?);
                Ok(patches)
            }
            // `a || b`: a truthy left makes the condition hold (skip the right); only the right's
            // falsiness exits.
            Expr::Binary {
                left,
                op: BinOp::Or,
                right,
                ..
            } => {
                self.compile_expr(left)?;
                let take_right = self.emit_jump(Opcode::JumpIfFalse); // left falsy → evaluate right
                let done = self.emit_jump(Opcode::Jump); // left truthy → condition holds
                self.patch_jump(take_right, self.chunk.code.len());
                let patches = self.compile_condition_jump_if_false(right)?;
                self.patch_jump(done, self.chunk.code.len());
                Ok(patches)
            }
            // Any other expression: evaluate it; one JumpIfFalse consumes the value.
            _ => {
                self.compile_expr(cond)?;
                Ok(vec![self.emit_jump(Opcode::JumpIfFalse)])
            }
        }
    }

    /// Patch a JumpBack operand: distance from the IP after this insn back to `target`.
    /// `patch_pos` is the first byte of the u16 operand (same as [`Self::emit_jump_back`]'s return value).
    fn patch_jump_back(&mut self, patch_pos: usize, target: usize) {
        let after_insn = patch_pos + 2;
        let dist = after_insn.saturating_sub(target);
        let bytes = (dist as u16).to_be_bytes();
        self.chunk.code[patch_pos] = bytes[0];
        self.chunk.code[patch_pos + 1] = bytes[1];
    }

    /// Detect property-based numeric sort: (a, b) => a.prop - b.prop or (a, b) => b.prop - a.prop.
    /// Returns Some((prop_name, asc)) or None.
    fn detect_property_sort_comparator(expr: &Expr) -> Option<(Arc<str>, bool)> {
        if let Expr::ArrowFunction { params, body, .. } = expr {
            if params.len() != 2 {
                return None;
            }
            let (param_a, param_b) = match (&params[0], &params[1]) {
                (FunParam::Simple(a), FunParam::Simple(b))
                    if a.default.is_none() && b.default.is_none() =>
                {
                    (a.name.as_ref(), b.name.as_ref())
                }
                _ => return None,
            };
            let body_expr = match body {
                ArrowBody::Expr(e) => e.as_ref(),
                ArrowBody::Block(stmt) => {
                    if let Statement::ExprStmt { expr: e, .. } = stmt.as_ref() {
                        e
                    } else {
                        return None;
                    }
                }
            };
            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = body_expr
            {
                if let (
                    Expr::Member {
                        object: lo,
                        prop: MemberProp::Name { name: p, .. },
                        ..
                    },
                    Expr::Member {
                        object: ro,
                        prop: MemberProp::Name { name: pr, .. },
                        ..
                    },
                ) = (left.as_ref(), right.as_ref())
                {
                    if p != pr {
                        return None;
                    }
                    if let (Expr::Ident { name: ln, .. }, Expr::Ident { name: rn, .. }) =
                        (lo.as_ref(), ro.as_ref())
                    {
                        if ln.as_ref() == param_a && rn.as_ref() == param_b {
                            return Some((Arc::clone(p), true));
                        }
                        if ln.as_ref() == param_b && rn.as_ref() == param_a {
                            return Some((Arc::clone(p), false));
                        }
                    }
                }
            }
        }
        None
    }

    /// Detect numeric sort comparator: (a, b) => a - b (asc) or (a, b) => b - a (desc).
    fn detect_numeric_sort_comparator(expr: &Expr) -> Option<bool> {
        if let Expr::ArrowFunction { params, body, .. } = expr {
            if params.len() != 2 {
                return None;
            }
            let (param_a, param_b) = match (&params[0], &params[1]) {
                (FunParam::Simple(a), FunParam::Simple(b))
                    if a.default.is_none() && b.default.is_none() =>
                {
                    (a.name.as_ref(), b.name.as_ref())
                }
                _ => return None,
            };
            let body_expr = match body {
                ArrowBody::Expr(e) => e.as_ref(),
                ArrowBody::Block(stmt) => {
                    if let Statement::ExprStmt { expr: e, .. } = stmt.as_ref() {
                        e
                    } else {
                        return None;
                    }
                }
            };
            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = body_expr
            {
                if let (
                    Expr::Ident {
                        name: left_name, ..
                    },
                    Expr::Ident {
                        name: right_name, ..
                    },
                ) = (left.as_ref(), right.as_ref())
                {
                    if left_name.as_ref() == param_a && right_name.as_ref() == param_b {
                        return Some(true);
                    }
                    if left_name.as_ref() == param_b && right_name.as_ref() == param_a {
                        return Some(false);
                    }
                }
            }
        }
        None
    }

    /// Detect simple map callback: x => x (identity) or x => x op const / x => const op x.
    /// Returns SimpleMapResult for map optimization.
    fn detect_simple_map_callback(expr: &Expr) -> Option<SimpleMapResult> {
        let (params, body) = match expr {
            Expr::ArrowFunction { params, body, .. } => (params, body),
            _ => return None,
        };
        if params.len() != 1 {
            return None;
        }
        let param_name = match &params[0] {
            FunParam::Simple(tp) if tp.default.is_none() => tp.name.as_ref(),
            _ => return None,
        };
        let expr_ref: &Expr = match body {
            ArrowBody::Expr(e) => e.as_ref(),
            ArrowBody::Block(stmt) => {
                let s = stmt.as_ref();
                if let Statement::Return {
                    value: Some(ref e), ..
                } = s
                {
                    e
                } else if let Statement::ExprStmt { expr: ref e, .. } = s {
                    e
                } else {
                    return None;
                }
            }
        };
        // Identity: x => x
        if let Expr::Ident { name, .. } = expr_ref {
            if name.as_ref() == param_name {
                return Some(SimpleMapResult::Identity);
            }
        }
        // Binary: x op const or const op x
        if let Expr::Binary {
            left, op, right, ..
        } = expr_ref
        {
            let left_is_param =
                matches!(left.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let right_is_param =
                matches!(right.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let left_is_literal = matches!(left.as_ref(), Expr::Literal { .. });
            let right_is_literal = matches!(right.as_ref(), Expr::Literal { .. });
            if left_is_param && right_is_literal {
                if let Some(c) = literal_to_constant(right.as_ref()) {
                    return Some(SimpleMapResult::BinOp(*op, c, true));
                }
            }
            if left_is_literal && right_is_param {
                if let Some(c) = literal_to_constant(left.as_ref()) {
                    return Some(SimpleMapResult::BinOp(*op, c, false));
                }
            }
        }
        None
    }

    /// Detect simple filter callback: x => x op const or x => const op x (comparison that returns bool).
    fn detect_simple_filter_callback(expr: &Expr) -> Option<(BinOp, Constant, bool)> {
        let (params, body) = match expr {
            Expr::ArrowFunction { params, body, .. } => (params, body),
            _ => return None,
        };
        if params.len() != 1 {
            return None;
        }
        let param_name = match &params[0] {
            FunParam::Simple(tp) if tp.default.is_none() => tp.name.as_ref(),
            _ => return None,
        };
        let expr_ref: &Expr = match body {
            ArrowBody::Expr(e) => e.as_ref(),
            ArrowBody::Block(stmt) => {
                let s = stmt.as_ref();
                if let Statement::Return {
                    value: Some(ref e), ..
                } = s
                {
                    e
                } else if let Statement::ExprStmt { expr: ref e, .. } = s {
                    e
                } else {
                    return None;
                }
            }
        };
        if let Expr::Binary {
            left, op, right, ..
        } = expr_ref
        {
            if !matches!(
                op,
                BinOp::Eq
                    | BinOp::Ne
                    | BinOp::StrictEq
                    | BinOp::StrictNe
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or
            ) {
                return None;
            }
            let left_is_param =
                matches!(left.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let right_is_param =
                matches!(right.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let left_is_literal = matches!(left.as_ref(), Expr::Literal { .. });
            let right_is_literal = matches!(right.as_ref(), Expr::Literal { .. });
            if left_is_param && right_is_literal {
                if let Some(c) = literal_to_constant(right.as_ref()) {
                    return Some((*op, c, true));
                }
            }
            if left_is_literal && right_is_param {
                if let Some(c) = literal_to_constant(left.as_ref()) {
                    return Some((*op, c, false));
                }
            }
        }
        None
    }

    fn compile_program(&mut self, program: &Program) -> Result<(), CompileError> {
        let stmts = &program.statements;
        // Top-level general slot-based locals — NON-REPL only (REPL persists top-level `let`s to
        // globals across lines, which slots can't do). Set up before compiling; the frame size is the
        // monotonic `next_slot` high-water, applied to the chunk after compilation.
        if !self.retain_last_expr {
            if let Some(cap) = slot_analyze_toplevel(stmts) {
                self.general_slots = true;
                self.slot_captured = cap;
                self.slot_scopes.push(HashMap::default());
            }
        }
        let last_is_expr = self.retain_last_expr
            && stmts
                .last()
                .map(|s| matches!(s, Statement::ExprStmt { .. }))
                .unwrap_or(false);

        if last_is_expr {
            let (rest, last) = stmts.split_at(stmts.len().saturating_sub(1));
            for stmt in rest {
                self.compile_statement(stmt)?;
            }
            if let Some(Statement::ExprStmt { expr, .. }) = last.first() {
                self.compile_expr(expr)?;
            }
        } else {
            for stmt in stmts {
                self.compile_statement(stmt)?;
            }
            let idx = self.constant_idx(Constant::Null);
            self.emit(Opcode::LoadConst);
            self.chunk.write_u16(idx);
        }
        // Apply the top-level slot frame size (only if any local was actually slotted).
        if self.general_slots && self.next_slot > 0 {
            self.chunk.slot_based = true;
            self.chunk.num_slots = self.next_slot;
        }
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        self.mark_line(stmt.span());
        match stmt {
            Statement::Block { statements, .. } => {
                self.emit(Opcode::EnterBlock);
                self.block_depth += 1;
                self.enter_block_scope();
                for s in statements {
                    self.compile_statement(s)?;
                }
                self.exit_block_scope();
                self.emit(Opcode::ExitBlock);
                self.block_depth -= 1;
            }
            // Comma-declarators: a transparent group — compile each declarator in
            // the *current* block scope (no EnterBlock/ExitBlock).
            Statement::Multi { statements, .. } => {
                for s in statements {
                    self.compile_statement(s)?;
                }
            }
            Statement::VarDecl {
                name,
                init,
                mutable: _,
                ..
            } => {
                if let Some(expr) = init {
                    self.compile_expr(expr)?;
                } else {
                    let idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                if self.general_slots && !self.slot_captured.contains(name.as_ref()) {
                    // Uncaptured local → allocate a fresh frame slot + write it directly.
                    let slot = self.declare_slot(name);
                    self.emit_u16(Opcode::StoreLocal, slot);
                } else {
                    let idx = self.name_idx(name);
                    self.emit_u16(Opcode::DeclareVar, idx);
                    self.scope
                        .last_mut()
                        .unwrap()
                        .insert(Arc::clone(name), false);
                }
            }
            Statement::VarDeclDestructure { pattern, init, .. } => {
                self.compile_expr(init)?;
                self.compile_destructure(pattern, false, false)?;
            }
            Statement::ExprStmt { expr, .. } => {
                // String-builder fast path: statement-position `acc += rhs` on a frame-slot local
                // compiles to `<rhs>; AppendLocal slot` (no LoadLocal/Dup/StoreLocal/Pop), letting
                // the VM append in amortized O(1) without materializing the discarded result. Only
                // for a simple slot-resolved identifier with the `+` compound op; everything else
                // (name-based vars, other ops) keeps the generic path.
                if let Expr::CompoundAssign {
                    name,
                    op: CompoundOp::Add,
                    value,
                    ..
                } = expr
                {
                    if let Some(slot) = self.resolve_slot(name) {
                        self.compile_expr(value)?;
                        self.emit_u16(Opcode::AppendLocal, slot);
                        return Ok(());
                    }
                }
                // Same builder fast path for the plain-assign spelling `s = s + <str>` — but ONLY when
                // the appended operand is PROVABLY a string. A numeric accumulator (`i = i + 1`) must
                // NOT builder-ize: the VM keeps a single string builder slot, so a second builder-ized
                // slot flushes the first every iteration → O(n²) (#186). Restricting to string-typed
                // RHS keeps `s = s + "x"` fast (string_concat) while `i = i + 1` stays a plain store.
                if let Expr::Assign { name, value, .. } = expr {
                    if let Expr::Binary {
                        left,
                        op: BinOp::Add,
                        right,
                        ..
                    } = value.as_ref()
                    {
                        if matches!(left.as_ref(), Expr::Ident { name: ln, .. } if ln == name)
                            && is_string_typed(right)
                        {
                            if let Some(slot) = self.resolve_slot(name) {
                                self.compile_expr(right)?;
                                self.emit_u16(Opcode::AppendLocal, slot);
                                return Ok(());
                            }
                        }
                    }
                }
                self.compile_expr(expr)?;
                self.emit(Opcode::Pop);
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let jump_else_sites = self.compile_condition_jump_if_false(cond)?;
                self.compile_statement(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                let else_target = self.chunk.code.len();
                for site in &jump_else_sites {
                    self.patch_jump(*site, else_target);
                }
                if let Some(else_s) = else_branch {
                    self.compile_statement(else_s)?;
                }
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Statement::While { cond, body, .. } => {
                // Per-iteration `let`: a `let` declared directly in the loop body is a fresh binding
                // each iteration, so a closure created in the body captures THIS iteration's value.
                // Register those names (same overlay mechanism as for/for-of loop vars).
                let body_lets = Self::loop_body_block_lets(body);
                for n in &body_lets {
                    let idx = self.name_idx(n);
                    self.emit_u16(Opcode::LoopVarsBegin, idx);
                }
                let start = self.chunk.code.len();
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                    continue_is_forward_jump: false,
                });
                self.breakable_stack.push(Breakable::Loop {
                    unwind_depth: self.block_depth,
                });
                // Condition-position lowering: empty stack at each JumpIfFalse keeps the loop
                // JIT-eligible (#167). Each returned site exits to `end`.
                let jump_out_sites = self.compile_condition_jump_if_false(cond)?;
                self.compile_statement(body)?;
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                for site in &jump_out_sites {
                    self.patch_jump(*site, end);
                }
                let info = self.loop_stack.pop().unwrap();
                self.breakable_stack.pop();
                for p in info.continue_patches {
                    self.patch_jump_back(p, start);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                for _ in &body_lets {
                    self.emit(Opcode::LoopVarsEnd);
                }
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                self.enter_block_scope();
                if let Some(i) = init {
                    self.compile_for_init_statement(i.as_ref())?;
                }
                // ES per-iteration `let`: register the loop var so a closure created in the body
                // captures THIS iteration's value (not the final one). One push per loop entry; the
                // per-iteration snapshot only happens when a closure is actually created, so
                // closure-free loops are unaffected.
                let loop_var: Option<Arc<str>> = match init.as_deref() {
                    Some(Statement::VarDecl { name, .. }) => Some(Arc::clone(name)),
                    _ => None,
                };
                if let Some(ref n) = loop_var {
                    let idx = self.name_idx(n);
                    self.emit_u16(Opcode::LoopVarsBegin, idx);
                }
                let cond_start = self.chunk.code.len();
                // Condition-position lowering keeps the loop JIT-eligible (#167). The absent-condition
                // `for (;;)` keeps its `true` constant + single exit jump.
                let jump_out_sites = if let Some(c) = cond {
                    self.compile_condition_jump_if_false(c)?
                } else {
                    let idx = self.constant_idx(Constant::Bool(true));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                    vec![self.emit_jump(Opcode::JumpIfFalse)]
                };
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                    continue_is_forward_jump: true,
                });
                self.breakable_stack.push(Breakable::Loop {
                    unwind_depth: self.block_depth,
                });
                self.compile_statement(body)?;
                let update_start = self.chunk.code.len();
                if let Some(u) = update {
                    self.compile_expr(u)?;
                    self.emit(Opcode::Pop);
                }
                let info = self.loop_stack.pop().unwrap();
                for p in info.continue_patches {
                    self.patch_jump(p, update_start);
                }
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(cond_start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                for site in &jump_out_sites {
                    self.patch_jump(*site, end);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                // After the loop fully exits (normal or break, both land at `end`): close the
                // per-iteration region.
                if loop_var.is_some() {
                    self.emit(Opcode::LoopVarsEnd);
                }
                self.breakable_stack.pop();
                self.exit_block_scope();
            }
            Statement::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                self.compile_expr(iterable)?;
                // Normalize a JS iterator object (Map/Set `.values()` etc.) to an array so the
                // index-based loop below can iterate it; arrays/strings pass through untouched.
                self.emit(Opcode::IterNormalize);
                self.enter_block_scope();
                let arr_name = Arc::from("__forof_arr__");
                let i_name = Arc::from("__forof_i__");
                let len_name = Arc::from("__forof_len__");
                let arr_idx = self.name_idx(&arr_name);
                let i_idx = self.name_idx(&i_name);
                let len_idx = self.name_idx(&len_name);
                let name_idx = self.name_idx(name);
                self.emit_u16(Opcode::DeclareVar, arr_idx);
                self.scope
                    .last_mut()
                    .unwrap()
                    .insert(arr_name.clone(), false);
                self.emit_u16(Opcode::LoadVar, arr_idx);
                let len_name_idx = self.name_idx(&Arc::from("length"));
                self.emit_u16(Opcode::GetMember, len_name_idx);
                self.emit_u16(Opcode::DeclareVar, len_idx);
                self.scope
                    .last_mut()
                    .unwrap()
                    .insert(len_name.clone(), false);
                let zero_idx = self.constant_idx(Constant::Number(0.0));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(zero_idx);
                self.emit_u16(Opcode::DeclareVar, i_idx);
                self.scope.last_mut().unwrap().insert(i_name.clone(), false);
                // ES per-iteration `let` for `for (let v of …)`: register the loop var so a closure
                // in the body captures this iteration's element (emitted once, before loop_start).
                self.emit_u16(Opcode::LoopVarsBegin, name_idx);
                // Pre-tested loop, like the C-style `for` above: test `i < len` at the TOP, before
                // reading `arr[i]`. A bottom-tested loop ran the body once on an empty array (reading
                // `arr[0]` → null) and spun forever on `continue` (which skipped the increment).
                let cond_start = self.chunk.code.len();
                self.emit_u16(Opcode::LoadVar, i_idx);
                self.emit_u16(Opcode::LoadVar, len_idx);
                self.emit_u8(Opcode::BinOp, 10);
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                    continue_is_forward_jump: true,
                });
                self.breakable_stack.push(Breakable::Loop {
                    unwind_depth: self.block_depth,
                });
                self.emit_u16(Opcode::LoadVar, arr_idx);
                self.emit_u16(Opcode::LoadVar, i_idx);
                self.emit(Opcode::GetIndex);
                self.emit_u16(Opcode::DeclareVar, name_idx);
                self.scope
                    .last_mut()
                    .unwrap()
                    .insert(Arc::clone(name), false);
                self.compile_statement(body)?;
                // `continue` lands here: increment `i`, then fall through to the JumpBack → re-test.
                let update_start = self.chunk.code.len();
                self.emit_u16(Opcode::LoadVar, i_idx);
                let one_idx = self.constant_idx(Constant::Number(1.0));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one_idx);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit_u16(Opcode::StoreVar, i_idx);
                let info = self.loop_stack.pop().unwrap();
                self.breakable_stack.pop();
                for p in info.continue_patches {
                    self.patch_jump(p, update_start);
                }
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(cond_start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_out, end);
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                self.emit(Opcode::LoopVarsEnd);
                self.exit_block_scope();
            }
            Statement::ForIn {
                name,
                name_span,
                object,
                body,
                span,
            } => {
                // Lower `for (let k in obj)` to `for (let k of Object.keys(obj))` (#413): Object.keys
                // yields exactly the own enumerable keys the interpreter enumerates — insertion order
                // for objects, index strings for arrays, and `[]` for a non-object (so `for (k in
                // null)` no-ops). Reuses the whole for-of iteration path.
                let dummy = Span {
                    start: (0, 0),
                    end: (0, 0),
                };
                let keys_call = Expr::Call {
                    callee: Box::new(Expr::Member {
                        object: Box::new(Expr::Ident {
                            name: Arc::from("Object"),
                            span: dummy,
                        }),
                        prop: MemberProp::Name {
                            name: Arc::from("keys"),
                            span: dummy,
                        },
                        optional: false,
                        span: dummy,
                    }),
                    args: vec![CallArg::Expr(object.clone())],
                    span: dummy,
                };
                let lowered = Statement::ForOf {
                    name: Arc::clone(name),
                    name_span: *name_span,
                    iterable: keys_call,
                    body: body.clone(),
                    span: *span,
                };
                self.compile_statement(&lowered)?;
            }
            Statement::Return { value, .. } => {
                // Evaluate the return value first (JS order), then run any enclosing `finally`
                // blocks (they're stack-neutral, so the value stays on top), then return.
                if let Some(v) = value {
                    self.compile_expr(v)?;
                } else {
                    let idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                self.emit_pending_finallys()?;
                self.emit(Opcode::Return);
            }
            Statement::Break { .. } => {
                let unwind_depth = match self.breakable_stack.last() {
                    Some(Breakable::Loop { unwind_depth })
                    | Some(Breakable::Switch { unwind_depth }) => *unwind_depth,
                    None => {
                        return Err(CompileError {
                            message: "break not inside a loop or switch".to_string(),
                        });
                    }
                };
                self.emit_exit_blocks_until_depth(unwind_depth);
                let pos = self.emit_jump(Opcode::Jump);
                match self.breakable_stack.last() {
                    Some(Breakable::Loop { .. }) => {
                        self.loop_stack.last_mut().unwrap().break_patches.push(pos);
                    }
                    Some(Breakable::Switch { .. }) => {
                        self.switch_stack
                            .last_mut()
                            .unwrap()
                            .break_patches
                            .push(pos);
                    }
                    None => {}
                }
            }
            Statement::Continue { .. } => {
                let unwind_depth = self
                    .breakable_stack
                    .iter()
                    .rev()
                    .find_map(|b| match b {
                        Breakable::Loop { unwind_depth } => Some(*unwind_depth),
                        Breakable::Switch { .. } => None,
                    })
                    .ok_or_else(|| CompileError {
                        message: "continue not inside a loop".to_string(),
                    })?;
                self.emit_exit_blocks_until_depth(unwind_depth);
                let forward = self
                    .loop_stack
                    .last()
                    .expect("continue not inside a loop")
                    .continue_is_forward_jump;
                let pos = if forward {
                    self.emit_jump(Opcode::Jump)
                } else {
                    self.emit_jump_back()
                };
                self.loop_stack
                    .last_mut()
                    .expect("continue not inside a loop")
                    .continue_patches
                    .push(pos);
            }
            Statement::FunDecl {
                name,
                params,
                body,
                rest_param,
                async_: _,
                ..
            } => {
                let formal_len = params.len();
                let (mut param_names, slots) = Self::plan_function_params(params)?;
                let simple_slots = simple_fn_slots(params, rest_param.is_some(), |pset| {
                    stmt_is_param_only(body, pset)
                });
                // Capture-aware general slot-based locals when the simple param-only fast path doesn't
                // apply. Gated by `TISH_VM_SLOTS` (off ⇒ None ⇒ byte-identical). Frame size is known only
                // AFTER compilation (slots are allocated as the body declares locals) → set on the chunk below.
                let captured = if simple_slots.is_none() {
                    slot_analyze(params, rest_param.is_some(), body)
                } else {
                    None
                };
                let mut inner = Chunk::new();
                inner.source = self.chunk.source.clone(); // propagate file for error locations (#74)
                if let Some(rp) = rest_param {
                    param_names.push(Arc::clone(&rp.name));
                    inner.rest_param_index = (param_names.len() as u16).saturating_sub(1);
                }
                for p in &param_names {
                    inner.add_name(Arc::clone(p));
                }
                inner.param_count = param_names.len() as u16;
                if simple_slots.is_some() {
                    inner.slot_based = true;
                    inner.num_slots = param_names.len() as u16;
                }
                // #187: stamp the chunk with its global name so the JIT can register it as a directly-
                // callable callee — but ONLY when it is a provably-stable top-level function. `self`
                // being the ROOT compiler (top-level) is indicated by `self.self_fn_name.is_none()` and
                // an empty `slot_scopes`; simplest sound check: the name is in `stable_globals` (which
                // by construction contains only the unique top-level `function name`).
                if self.stable_globals.contains(name) {
                    inner.global_name = Some(Arc::clone(name));
                }
                let mut inner_comp = Compiler::new(&mut inner, false);
                inner_comp.stable_globals = Arc::clone(&self.stable_globals);
                // #203: `Math` unshadowed is a whole-program property; thread it so `Math.<fn>` in this
                // function body lowers to the MathUnary intrinsic (else the numeric JIT bails).
                inner_comp.math_is_global = self.math_is_global;
                // Recursion-JIT enabler: if `name`'s binding is provably stable in the body (no
                // param shadows it, no reassignment/redeclaration), direct `name(args)` calls inside
                // compile to `SelfCall` — no name lookup, and the numeric JIT lowers it to a native
                // recursive call. Conservative `stmt_rebinds` errs toward NOT enabling (safe).
                if !params_bind_name(params, name.as_ref()) && !stmt_rebinds(body, name.as_ref()) {
                    inner_comp.self_fn_name = Some(Arc::clone(name));
                }
                let mut general_frame_slots: Option<u16> = None;
                if let Some(map) = simple_slots {
                    inner_comp.slot_ctx = Some(map);
                    inner_comp.emit_param_defaults_prologue(params)?;
                    inner_comp.compile_statement(body)?;
                } else if let Some(cap) = captured {
                    // Params (all uncaptured — gated) → slots 0..n (matching the VM's param binding);
                    // uncaptured body `let`s get fresh slots via the scope-aware allocator; captured
                    // locals stay name-based in `local_scope` (which closures capture).
                    inner_comp.general_slots = true;
                    inner_comp.slot_captured = cap;
                    inner_comp.slot_scopes.push(HashMap::new());
                    for p in &param_names {
                        inner_comp.declare_slot(p);
                    }
                    inner_comp.emit_param_defaults_prologue(params)?;
                    inner_comp.compile_statement(body)?;
                    general_frame_slots = Some(inner_comp.next_slot);
                } else {
                    inner_comp.scope = vec![param_names
                        .iter()
                        .map(|n| (Arc::clone(n), false))
                        .collect::<HashMap<_, _>>()];
                    inner_comp
                        .emit_param_destructure_prologue(&param_names[..formal_len], &slots)?;
                    inner_comp.emit_param_defaults_prologue(params)?;
                    inner_comp.compile_statement(body)?;
                }
                inner_comp.emit(Opcode::LoadConst);
                let idx = inner_comp.constant_idx(Constant::Null);
                inner_comp.chunk.write_u16(idx);
                inner_comp.emit(Opcode::Return);
                if let Some(n) = general_frame_slots {
                    inner_comp.chunk.slot_based = true;
                    inner_comp.chunk.num_slots = n;
                }
                let nested_idx = self.chunk.add_nested(inner);
                self.emit(Opcode::LoadConst);
                let idx = self.constant_idx(Constant::Closure(nested_idx));
                self.chunk.write_u16(idx);
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::DeclareVar, idx);
                self.scope
                    .last_mut()
                    .unwrap()
                    .insert(Arc::clone(name), false);
            }
            Statement::DoWhile { body, cond, .. } => {
                let body_lets = Self::loop_body_block_lets(body);
                for n in &body_lets {
                    let idx = self.name_idx(n);
                    self.emit_u16(Opcode::LoopVarsBegin, idx);
                }
                let start = self.chunk.code.len();
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                    // `continue` jumps to the condition test, which is emitted AFTER the body — a
                    // FORWARD jump. (A backward JumpBack would patch to dist = 0 via saturating_sub
                    // of a forward target, becoming a no-op; execution then fell through and re-ran
                    // the body's already-unwound ExitBlock on an empty block stack — the
                    // "ExitBlock without matching EnterBlock" crash.)
                    continue_is_forward_jump: true,
                });
                self.breakable_stack.push(Breakable::Loop {
                    unwind_depth: self.block_depth,
                });
                self.compile_statement(body)?;
                let cond_start = self.chunk.code.len();
                // Condition-position lowering: a false condition exits to `end`, a true one falls
                // through to the JumpBack — JIT-eligible, no value left on the stack (#167).
                let exit_sites = self.compile_condition_jump_if_false(cond)?;
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                for site in &exit_sites {
                    self.patch_jump(*site, end);
                }
                let info = self.loop_stack.pop().unwrap();
                self.breakable_stack.pop();
                for p in info.continue_patches {
                    // Forward jump to the condition (see continue_is_forward_jump above).
                    self.patch_jump(p, cond_start);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                for _ in &body_lets {
                    self.emit(Opcode::LoopVarsEnd);
                }
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                let switch_unwind_depth = self.block_depth;
                self.switch_stack.push(SwitchInfo {
                    break_patches: Vec::new(),
                });
                self.breakable_stack.push(Breakable::Switch {
                    unwind_depth: switch_unwind_depth,
                });
                self.compile_expr(expr)?;
                self.emit(Opcode::Dup);
                let mut end_patches = Vec::new();
                for (case_expr, case_body) in cases {
                    self.emit(Opcode::Dup);
                    if let Some(ce) = case_expr {
                        self.compile_expr(ce)?;
                        self.emit_u8(Opcode::BinOp, 8);
                        let jump_next = self.emit_jump(Opcode::JumpIfFalse);
                        // JumpIfFalse already pops the match result when taking this case
                        self.compile_statement(&Statement::Block {
                            statements: case_body.clone(),
                            span: Span {
                                start: (0, 0),
                                end: (0, 0),
                            },
                        })?;
                        let jump_end = self.emit_jump(Opcode::Jump);
                        end_patches.push(jump_end);
                        self.patch_jump(jump_next, self.chunk.code.len());
                    } else {
                        self.emit(Opcode::Pop);
                        self.compile_statement(&Statement::Block {
                            statements: case_body.clone(),
                            span: Span {
                                start: (0, 0),
                                end: (0, 0),
                            },
                        })?;
                    }
                }
                if let Some(body) = default_body {
                    self.emit(Opcode::Pop);
                    self.compile_statement(&Statement::Block {
                        statements: body.clone(),
                        span: Span {
                            start: (0, 0),
                            end: (0, 0),
                        },
                    })?;
                } else {
                    self.emit(Opcode::Pop);
                }
                for p in end_patches {
                    self.patch_jump(p, self.chunk.code.len());
                }
                let sw = self.switch_stack.pop().unwrap();
                self.breakable_stack.pop();
                for p in sw.break_patches {
                    self.patch_jump(p, self.chunk.code.len());
                }
            }
            Statement::Throw { value, .. } => {
                self.compile_expr(value)?;
                self.emit(Opcode::Throw);
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                finally_body,
                ..
            } => {
                let catch_offset_pos = self.chunk.code.len();
                self.emit(Opcode::EnterTry);
                self.chunk.write_u16(0);
                // A `return` inside the body/catch must run this finally on the way out.
                if let Some(f) = finally_body {
                    self.finally_stack.push((**f).clone());
                }
                self.compile_statement(body)?;
                self.emit(Opcode::ExitTry);
                let jump_over_catch = self.emit_jump(Opcode::Jump);
                let catch_start = self.chunk.code.len();
                if let Some(catch_stmt) = catch_body {
                    if let Some(param) = catch_param {
                        self.emit(Opcode::EnterBlock);
                        self.block_depth += 1;
                        self.enter_block_scope();
                        let param_idx = self.name_idx(param);
                        self.emit_u16(Opcode::DeclareVar, param_idx);
                        self.scope
                            .last_mut()
                            .unwrap()
                            .insert(Arc::clone(param), false);
                        self.compile_statement(catch_stmt)?;
                        self.exit_block_scope();
                        self.emit(Opcode::ExitBlock);
                        self.block_depth -= 1;
                    } else {
                        self.emit(Opcode::Pop);
                        self.compile_statement(catch_stmt)?;
                    }
                } else {
                    // No catch: run `finally` on the exception path, then re-raise (propagate).
                    if let Some(f) = finally_body {
                        self.compile_statement(f)?;
                    }
                    self.emit(Opcode::Throw);
                }
                let after_catch = self.chunk.code.len();
                self.patch_jump(jump_over_catch, after_catch);
                // The finally is no longer pending for enclosing returns once we emit its inline
                // (normal-path) copy below.
                if finally_body.is_some() {
                    self.finally_stack.pop();
                }
                if let Some(finally) = finally_body {
                    self.compile_statement(finally)?;
                }
                let catch_offset =
                    catch_start.wrapping_sub(catch_offset_pos).wrapping_sub(3) as u16;
                self.chunk.code[catch_offset_pos + 1] = (catch_offset >> 8) as u8;
                self.chunk.code[catch_offset_pos + 2] = (catch_offset & 0xff) as u8;
            }
            Statement::Import { .. } => {
                return Err(CompileError {
                    message: "Import not supported in bytecode".to_string(),
                });
            }
            Statement::Export { declaration, .. } => match declaration.as_ref() {
                ExportDeclaration::Named(inner_stmt) => {
                    self.compile_statement(inner_stmt.as_ref())?;
                }
                ExportDeclaration::Default(_) => {
                    return Err(CompileError {
                        message: "export default is not supported in bytecode".to_string(),
                    });
                }
                ExportDeclaration::ReExport { .. } => {}
            },
            Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => {}
        }
        Ok(())
    }

    fn compile_destructure(
        &mut self,
        pattern: &DestructPattern,
        mutable: bool,
        for_header_binding: bool,
    ) -> Result<(), CompileError> {
        let decl_op = if for_header_binding {
            Opcode::DeclareVarPlain
        } else {
            Opcode::DeclareVar
        };
        match pattern {
            DestructPattern::Array(elements) => {
                for (i, elem) in elements.iter().enumerate() {
                    match elem {
                        Some(DestructElement::Ident(name, _)) => {
                            self.emit(Opcode::Dup);
                            let idx = self.constant_idx(Constant::Number(i as f64));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.emit(Opcode::GetIndex);
                            let idx = self.name_idx(name);
                            self.emit_u16(decl_op, idx);
                            self.scope
                                .last_mut()
                                .unwrap()
                                .insert(Arc::clone(name), false);
                        }
                        // Array hole `[a, , c]`: position is skipped, no binding emitted.
                        None => {}
                        // Nested pattern `[[a, b], c]` or `[{x}, y]`: push source[i] and recurse.
                        // compile_destructure is stack-balanced (consumes exactly the value it
                        // destructures), so the source array beneath stays intact.
                        Some(DestructElement::Pattern(sub)) => {
                            self.emit(Opcode::Dup);
                            let idx = self.constant_idx(Constant::Number(i as f64));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.emit(Opcode::GetIndex);
                            self.compile_destructure(sub, mutable, for_header_binding)?;
                        }
                        // Rest `[a, ...rest]`: rest = source.slice(i). Use GetMember (not GetIndex)
                        // so the array's `slice` method resolves via get_member; GetIndex rejects
                        // string keys on arrays.
                        Some(DestructElement::Rest(name, _)) => {
                            self.emit(Opcode::Dup);
                            let slice_idx = self.name_idx(&Arc::from("slice"));
                            self.emit_u16(Opcode::GetMember, slice_idx);
                            let idx = self.constant_idx(Constant::Number(i as f64));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.emit_u16(Opcode::Call, 1);
                            let nidx = self.name_idx(name);
                            self.emit_u16(decl_op, nidx);
                            self.scope
                                .last_mut()
                                .unwrap()
                                .insert(Arc::clone(name), false);
                        }
                    }
                }
                self.emit(Opcode::Pop);
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    self.emit(Opcode::Dup);
                    let key_idx = self.constant_idx(Constant::String(Arc::clone(&prop.key)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(key_idx);
                    self.emit(Opcode::GetIndex); // GetIndex pops obj, index and uses get_member
                    match &prop.value {
                        DestructElement::Ident(name, _) => {
                            let idx = self.name_idx(name);
                            self.emit_u16(decl_op, idx);
                            if mutable {
                                self.scope
                                    .last_mut()
                                    .unwrap()
                                    .insert(Arc::clone(name), false);
                            }
                        }
                        // Nested value `{ outer: { inner } }` or `{ arr: [a, b] }`: obj[key] is
                        // already on the stack (GetIndex above); recurse to destructure it.
                        DestructElement::Pattern(sub) => {
                            self.compile_destructure(sub, mutable, for_header_binding)?;
                        }
                        // `{ ...rest }` needs the set of *remaining* keys; not yet supported.
                        DestructElement::Rest(_, _) => {
                            return Err(CompileError {
                                message: "Object rest destructuring not yet supported".to_string(),
                            });
                        }
                    }
                }
                self.emit(Opcode::Pop);
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        self.mark_line(expr.span());
        match expr {
            Expr::Literal { value, .. } => {
                let c = match value {
                    Literal::Number(n) => Constant::Number(*n),
                    Literal::String(s) => Constant::String(Arc::clone(s)),
                    Literal::Bool(b) => Constant::Bool(*b),
                    Literal::Null => Constant::Null,
                };
                let idx = self.constant_idx(c);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(idx);
            }
            Expr::Ident { name, .. } => {
                // `resolve_slot` checks BOTH the simple param-only map and the general scope stack.
                self.emit_var_load(name);
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                match op {
                    BinOp::And => {
                        // Short-circuit + value-returning (JS): `a && b` is `a` when `a` is falsy,
                        // else `b` — NOT a coerced boolean (#240). Mirror the `||` lowering below:
                        // keep the left operand on a falsy short-circuit, discard it and evaluate the
                        // right otherwise. (The old `BinOp(And)` here coerced a truthy-left result to
                        // a Bool, so `five() && 7` yielded `true` instead of `7`.)
                        self.compile_expr(left)?;
                        self.emit(Opcode::Dup);
                        let jump_end = self.emit_jump(Opcode::JumpIfFalse); // a falsy → keep a
                        self.emit(Opcode::Pop); // a truthy → discard a …
                        self.compile_expr(right)?; // … and yield b
                        self.patch_jump(jump_end, self.chunk.code.len());
                    }
                    BinOp::Or => {
                        // Short-circuit: a || b => if a then a else b
                        self.compile_expr(left)?;
                        self.emit(Opcode::Dup);
                        let jump_eval_right = self.emit_jump(Opcode::JumpIfFalse);
                        let jump_end = self.emit_jump(Opcode::Jump);
                        self.patch_jump(jump_eval_right, self.chunk.code.len());
                        self.emit(Opcode::Pop); // discard falsy left
                        self.compile_expr(right)?;
                        self.patch_jump(jump_end, self.chunk.code.len());
                    }
                    _ => {
                        self.compile_expr(left)?;
                        self.compile_expr(right)?;
                        self.emit_u8(Opcode::BinOp, binop_to_u8(*op));
                    }
                }
            }
            Expr::Unary { op, operand, .. } => {
                self.compile_expr(operand)?;
                self.emit_u8(Opcode::UnaryOp, unaryop_to_u8(*op));
            }
            Expr::Call { callee, args, .. } => {
                // #186: `Math.<unaryfn>(arg)` → `<arg>; MathUnary(id)` when `Math` is provably the
                // global builtin (unshadowed program-wide) and there is exactly one argument. Lets the
                // numeric JIT lower the intrinsic (math_trig; a Math.* prereq for other kernels).
                if self.math_is_global && args.len() == 1 {
                    if let Expr::Member {
                        object,
                        prop: MemberProp::Name { name: fname, .. },
                        optional: false,
                        ..
                    } = callee.as_ref()
                    {
                        if let (Expr::Ident { name: obj, .. }, CallArg::Expr(arg)) =
                            (object.as_ref(), &args[0])
                        {
                            if obj.as_ref() == "Math" && self.resolve_slot("Math").is_none() {
                                if let Some(mfn) = MathUnaryFn::from_name(fname.as_ref()) {
                                    self.compile_expr(arg)?;
                                    self.emit_u16(Opcode::MathUnary, mfn as u16);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                // #203: `Math.<binfn>(a, b)` → `<a>; <b>; MathBinary(id)` — the same unshadowed-global
                // gate, for exactly two positional args (variadic `Math.max(a,b,c)` falls through to a
                // generic call). Lets the numeric JIT lower clamp/pow kernels instead of bailing.
                if self.math_is_global && args.len() == 2 {
                    if let Expr::Member {
                        object,
                        prop: MemberProp::Name { name: fname, .. },
                        optional: false,
                        ..
                    } = callee.as_ref()
                    {
                        if let (Expr::Ident { name: obj, .. }, CallArg::Expr(a0), CallArg::Expr(a1)) =
                            (object.as_ref(), &args[0], &args[1])
                        {
                            if obj.as_ref() == "Math" && self.resolve_slot("Math").is_none() {
                                if let Some(bfn) = MathBinaryFn::from_name(fname.as_ref()) {
                                    self.compile_expr(a0)?;
                                    self.compile_expr(a1)?;
                                    self.emit_u16(Opcode::MathBinary, bfn as u16);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                // Fast path: arr.sort((a,b)=>a-b) or arr.sort((a,b)=>b-a) -> ArraySortNumeric
                if !args.iter().any(|a| matches!(a, CallArg::Spread(_)))
                    && args.len() == 1
                    && matches!(args[0], CallArg::Expr(_))
                {
                    if let (
                        Expr::Member {
                            object,
                            prop: MemberProp::Name { name: key, .. },
                            optional: false,
                            ..
                        },
                        CallArg::Expr(cmp_expr),
                    ) = (callee.as_ref(), &args[0])
                    {
                        if key.as_ref() == "sort" {
                            if let Some(ascending) = Self::detect_numeric_sort_comparator(cmp_expr)
                            {
                                self.compile_expr(object)?;
                                self.emit_u8(
                                    Opcode::ArraySortNumeric,
                                    if ascending { 0 } else { 1 },
                                );
                                return Ok(());
                            }
                            if let Some((prop, ascending)) =
                                Self::detect_property_sort_comparator(cmp_expr)
                            {
                                self.compile_expr(object)?;
                                let prop_idx = self.constant_idx(Constant::String(prop));
                                self.emit(Opcode::ArraySortByProperty);
                                self.chunk.write_u16(prop_idx);
                                self.chunk.write_u16(if ascending { 0 } else { 1 });
                                return Ok(());
                            }
                        }
                        if key.as_ref() == "map" {
                            if let Some(simple) = Self::detect_simple_map_callback(cmp_expr) {
                                self.compile_expr(object)?;
                                match simple {
                                    SimpleMapResult::Identity => {
                                        self.emit(Opcode::ArrayMapIdentity);
                                    }
                                    SimpleMapResult::BinOp(op, c, param_left) => {
                                        let const_idx = self.constant_idx(c);
                                        self.emit(Opcode::ArrayMapBinOp);
                                        self.chunk.write_u8(binop_to_u8(op));
                                        self.chunk.write_u16(const_idx);
                                        self.chunk.write_u8(if param_left { 0 } else { 1 });
                                    }
                                }
                                return Ok(());
                            }
                        }
                        if key.as_ref() == "filter" {
                            if let Some((op, const_val, param_left)) =
                                Self::detect_simple_filter_callback(cmp_expr)
                            {
                                self.compile_expr(object)?;
                                let const_idx = self.constant_idx(const_val);
                                self.emit(Opcode::ArrayFilterBinOp);
                                self.chunk.write_u8(binop_to_u8(op));
                                self.chunk.write_u16(const_idx);
                                self.chunk.write_u8(if param_left { 0 } else { 1 });
                                return Ok(());
                            }
                        }
                    }
                }
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if has_spread {
                    // Build args array [a, ...b, c], then callee, then CallSpread
                    self.emit_u16(Opcode::NewArray, 0);
                    for arg in args {
                        match arg {
                            CallArg::Expr(e) => {
                                self.compile_expr(e)?;
                                self.emit_u16(Opcode::NewArray, 1);
                                self.emit(Opcode::ConcatArray);
                            }
                            CallArg::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::ConcatArray);
                            }
                        }
                    }
                    self.compile_expr(callee)?;
                    self.emit(Opcode::CallSpread);
                } else {
                    // Self-recursion fast path: `name(args)` where `name` is this function's own
                    // provably-stable binding → `SelfCall` (no callee LoadVar, no closure dispatch;
                    // the JIT lowers it to a native recursive call). `self_fn_name` is only `Some`
                    // when the compiler proved `name` isn't shadowed or rebound (see FunDecl).
                    let is_self_call = matches!(
                        callee.as_ref(),
                        Expr::Ident { name, .. } if self.self_fn_name.as_deref() == Some(name.as_ref())
                    );
                    if is_self_call {
                        for arg in args {
                            if let CallArg::Expr(e) = arg {
                                self.compile_expr(e)?;
                            }
                        }
                        self.emit_u16(Opcode::SelfCall, args.len() as u16);
                    } else {
                        self.compile_expr(callee)?;
                        for arg in args {
                            if let CallArg::Expr(e) = arg {
                                self.compile_expr(e)?;
                            }
                        }
                        self.emit_u16(Opcode::Call, args.len() as u16);
                    }
                }
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                self.compile_expr(object)?;
                if *optional {
                    self.emit(Opcode::Dup);
                    let null_idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(null_idx);
                    self.emit_u8(Opcode::BinOp, 8);
                    let jump_to_null = self.emit_jump(Opcode::JumpIfFalse);
                    let jump_to_get_instr = self.chunk.code.len();
                    let jump_to_get = self.emit_jump(Opcode::Jump);
                    self.patch_jump(jump_to_null, jump_to_get_instr);
                    self.emit(Opcode::Pop);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(null_idx);
                    let jump_end = self.emit_jump(Opcode::Jump);
                    self.patch_jump(jump_to_get, self.chunk.code.len());
                    match prop {
                        MemberProp::Name { name: key, .. } => {
                            let idx = self.name_idx(key);
                            self.emit_u16(Opcode::GetMemberOptional, idx);
                        }
                        MemberProp::Expr(e) => {
                            self.compile_expr(e)?;
                            self.emit(Opcode::GetIndex);
                        }
                    }
                    self.patch_jump(jump_end, self.chunk.code.len());
                } else {
                    match prop {
                        MemberProp::Name { name: key, .. } => {
                            let idx = self.name_idx(key);
                            self.emit_u16(Opcode::GetMember, idx);
                        }
                        MemberProp::Expr(e) => {
                            self.compile_expr(e)?;
                            self.emit(Opcode::GetIndex);
                        }
                    }
                }
            }
            Expr::Index { object, index, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Opcode::GetIndex);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.compile_expr(cond)?;
                let jump_else = self.emit_jump(Opcode::JumpIfFalse);
                // JumpIfFalse pops condition when taking then; when taking else it also pops
                self.compile_expr(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_else, self.chunk.code.len());
                // no Pop: condition was already popped by JumpIfFalse
                self.compile_expr(else_branch)?;
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.compile_expr(left)?;
                self.emit(Opcode::Dup);
                let idx = self.constant_idx(Constant::Null);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(idx);
                self.emit_u8(Opcode::BinOp, binop_to_u8(BinOp::StrictNe));
                let jump_to_right = self.emit_jump(Opcode::JumpIfFalse);
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_to_right, self.chunk.code.len());
                self.emit(Opcode::Pop);
                self.compile_expr(right)?;
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Expr::Array { elements, .. } => {
                let has_spread = elements
                    .iter()
                    .any(|e| matches!(e, ArrayElement::Spread(_)));
                if has_spread {
                    // Build array incrementally: start with [], concat each element
                    self.emit_u16(Opcode::NewArray, 0);
                    for elem in elements {
                        match elem {
                            ArrayElement::Expr(e) => {
                                self.compile_expr(e)?;
                                self.emit_u16(Opcode::NewArray, 1);
                                self.emit(Opcode::ConcatArray);
                            }
                            ArrayElement::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::ConcatArray);
                            }
                        }
                    }
                } else {
                    for elem in elements {
                        if let ArrayElement::Expr(e) = elem {
                            self.compile_expr(e)?;
                        }
                    }
                    self.emit_u16(Opcode::NewArray, elements.len() as u16);
                }
            }
            Expr::Object { props, .. } => {
                let has_spread = props.iter().any(|p| matches!(p, ObjectProp::Spread(_)));
                if has_spread {
                    self.emit_u16(Opcode::NewObject, 0); // start with {}
                    for prop in props {
                        match prop {
                            ObjectProp::KeyValue(k, v, _) => {
                                let idx = self.constant_idx(Constant::String(Arc::clone(k)));
                                self.emit(Opcode::LoadConst);
                                self.chunk.write_u16(idx);
                                self.compile_expr(v)?;
                                self.emit_u16(Opcode::NewObject, 1);
                                self.emit(Opcode::MergeObject);
                            }
                            ObjectProp::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::MergeObject);
                            }
                        }
                    }
                } else {
                    for prop in props {
                        if let ObjectProp::KeyValue(k, v, _) = prop {
                            let idx = self.constant_idx(Constant::String(Arc::clone(k)));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.compile_expr(v)?;
                        }
                    }
                    self.emit_u16(Opcode::NewObject, props.len() as u16);
                }
            }
            Expr::Assign { name, value, .. } => {
                self.compile_expr(value)?;
                self.emit_var_store(name);
                self.emit_var_load(name); // assign yields value
            }
            Expr::TypeOf { operand, .. } => {
                let typeof_idx = self.name_idx(&Arc::from("typeof"));
                self.emit_u16(Opcode::LoadGlobal, typeof_idx);
                self.compile_expr(operand)?;
                self.emit_u16(Opcode::Call, 1);
            }
            Expr::ArrowFunction { params, body, .. } => {
                let formal_len = params.len();
                let (param_names, slots) = Self::plan_function_params(params)?;
                let simple_slots = simple_fn_slots(params, false, |pset| match body {
                    ArrowBody::Expr(e) => expr_is_param_only(e, pset),
                    ArrowBody::Block(s) => stmt_is_param_only(s, pset),
                });
                let mut inner = Chunk::new();
                inner.source = self.chunk.source.clone(); // propagate file for error locations (#74)
                for p in &param_names {
                    inner.add_name(Arc::clone(p));
                }
                inner.param_count = param_names.len() as u16;
                if simple_slots.is_some() {
                    inner.slot_based = true;
                    inner.num_slots = param_names.len() as u16;
                }
                let mut inner_comp = Compiler::new(&mut inner, false);
                inner_comp.stable_globals = Arc::clone(&self.stable_globals); // #187
                inner_comp.math_is_global = self.math_is_global; // #203 (see other call site)
                if let Some(map) = simple_slots {
                    inner_comp.slot_ctx = Some(map);
                } else {
                    inner_comp.scope = vec![param_names
                        .iter()
                        .map(|n| (Arc::clone(n), false))
                        .collect::<HashMap<_, _>>()];
                    inner_comp
                        .emit_param_destructure_prologue(&param_names[..formal_len], &slots)?;
                }
                inner_comp.emit_param_defaults_prologue(params)?;
                match body {
                    ArrowBody::Expr(e) => {
                        inner_comp.compile_expr(e)?;
                        inner_comp.emit(Opcode::Return);
                    }
                    ArrowBody::Block(s) => {
                        inner_comp.compile_statement(s)?;
                        let idx = inner_comp.constant_idx(Constant::Null);
                        inner_comp.emit(Opcode::LoadConst);
                        inner_comp.chunk.write_u16(idx);
                        inner_comp.emit(Opcode::Return);
                    }
                }
                let nested_idx = self.chunk.add_nested(inner);
                let idx = self.constant_idx(Constant::Closure(nested_idx));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(idx);
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                if exprs.is_empty() {
                    let s = quasis[0].to_string();
                    let idx = self.constant_idx(Constant::String(Arc::from(s)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                } else {
                    // Interleave quasis and exprs: quasi[0] + expr[0] + quasi[1] + expr[1] + ... + quasi[n]
                    let first = quasis[0].to_string();
                    let idx = self.constant_idx(Constant::String(Arc::from(first)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                    for (i, expr) in exprs.iter().enumerate() {
                        self.compile_expr(expr)?;
                        self.emit_u8(Opcode::BinOp, 0); // Add (string concat)
                        let quasi_s = quasis[i + 1].to_string();
                        let qidx = self.constant_idx(Constant::String(Arc::from(quasi_s)));
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(qidx);
                        self.emit_u8(Opcode::BinOp, 0); // Add
                    }
                }
            }
            Expr::PostfixInc { name, .. } => {
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_var_load(name);
                self.emit(Opcode::Dup);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit_var_store(name);
            }
            Expr::PostfixDec { name, .. } => {
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_var_load(name);
                self.emit(Opcode::Dup);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 1);
                self.emit_var_store(name);
            }
            Expr::PrefixInc { name, .. } => {
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_var_load(name);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit(Opcode::Dup);
                self.emit_var_store(name);
            }
            Expr::PrefixDec { name, .. } => {
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_var_load(name);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 1);
                self.emit(Opcode::Dup);
                self.emit_var_store(name);
            }
            Expr::CompoundAssign {
                name, op, value, ..
            } => {
                self.emit_var_load(name);
                self.compile_expr(value)?;
                self.emit_u8(Opcode::BinOp, compound_op_to_u8(*op));
                self.emit(Opcode::Dup);
                self.emit_var_store(name);
            }
            Expr::MemberAssign {
                object,
                prop,
                value,
                ..
            } => {
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                let idx = self.name_idx(prop);
                self.emit_u16(Opcode::SetMember, idx); // SetMember pops obj, val and pushes val back
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.compile_expr(value)?;
                self.emit(Opcode::Dup); // leave copy for assignment expression result
                self.emit(Opcode::SetIndex);
            }
            Expr::NativeModuleLoad {
                spec, export_name, ..
            } => {
                let spec_idx = self.constant_idx(Constant::String(Arc::clone(spec)));
                let export_idx = self.constant_idx(Constant::String(Arc::clone(export_name)));
                self.emit(Opcode::LoadNativeExport);
                self.chunk.write_u16(spec_idx);
                self.chunk.write_u16(export_idx);
            }
            Expr::JsxElement {
                tag,
                props,
                children,
                ..
            } => {
                self.compile_jsx_element(tag, props, children)?;
            }
            Expr::JsxFragment { children, .. } => {
                self.compile_jsx_fragment(children)?;
            }
            Expr::Await { operand, .. } => {
                // await expr => evaluate operand, then VM Opcode::AwaitPromise (throw on reject).
                self.compile_expr(operand)?;
                self.emit(Opcode::AwaitPromise);
            }
            Expr::Delete { target, .. } => {
                // `delete obj.prop` / `delete obj[key]` → push [obj, key], then DeleteIndex
                // pops both, removes the property, and pushes `true`. Deleting anything that
                // isn't a property reference is a no-op that still yields `true` (JS).
                match target.as_ref() {
                    Expr::Member {
                        object,
                        prop: MemberProp::Name { name, .. },
                        ..
                    } => {
                        self.compile_expr(object)?;
                        let idx = self.constant_idx(Constant::String(Arc::clone(name)));
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(idx);
                        self.emit(Opcode::DeleteIndex);
                    }
                    Expr::Member {
                        object,
                        prop: MemberProp::Expr(key),
                        ..
                    } => {
                        self.compile_expr(object)?;
                        self.compile_expr(key)?;
                        self.emit(Opcode::DeleteIndex);
                    }
                    Expr::Index { object, index, .. } => {
                        self.compile_expr(object)?;
                        self.compile_expr(index)?;
                        self.emit(Opcode::DeleteIndex);
                    }
                    _ => {
                        let idx = self.constant_idx(Constant::Bool(true));
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(idx);
                    }
                }
            }
            Expr::LogicalAssign {
                name, op, value, ..
            } => {
                match op {
                    LogicalAssignOp::OrOr => {
                        // ||= : if current is truthy, keep it; else eval rhs, assign, yield rhs
                        self.emit_var_load(name);
                        self.emit(Opcode::Dup);
                        let j_rhs = self.emit_jump(Opcode::JumpIfFalse);
                        let j_end = self.emit_jump(Opcode::Jump);
                        self.patch_jump(j_rhs, self.chunk.code.len());
                        self.emit(Opcode::Pop);
                        self.compile_expr(value)?;
                        self.emit_var_store(name);
                        self.emit_var_load(name);
                        let end = self.chunk.code.len();
                        self.patch_jump(j_end, end);
                    }
                    LogicalAssignOp::AndAnd => {
                        // &&= : if current is falsy, keep it; else eval rhs, assign, yield rhs
                        self.emit_var_load(name);
                        self.emit(Opcode::Dup);
                        let j_short = self.emit_jump(Opcode::JumpIfFalse);
                        self.emit(Opcode::Pop);
                        self.compile_expr(value)?;
                        self.emit_var_store(name);
                        self.emit_var_load(name);
                        let j_end = self.emit_jump(Opcode::Jump);
                        let end = self.chunk.code.len();
                        self.patch_jump(j_short, end);
                        self.patch_jump(j_end, end);
                    }
                    LogicalAssignOp::Nullish => {
                        // ??= : assign only when current === null (matches interpreter)
                        let null_c = self.constant_idx(Constant::Null);
                        self.emit_var_load(name);
                        self.emit(Opcode::Dup);
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(null_c);
                        self.emit_u8(Opcode::BinOp, binop_to_u8(BinOp::StrictEq));
                        let j_not_null = self.emit_jump(Opcode::JumpIfFalse);
                        self.emit(Opcode::Pop);
                        self.compile_expr(value)?;
                        self.emit_var_store(name);
                        self.emit_var_load(name);
                        let j_end = self.emit_jump(Opcode::Jump);
                        let end = self.chunk.code.len();
                        self.patch_jump(j_not_null, end);
                        self.patch_jump(j_end, end);
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if has_spread {
                    self.emit_u16(Opcode::NewArray, 0);
                    for arg in args {
                        match arg {
                            CallArg::Expr(e) => {
                                self.compile_expr(e)?;
                                self.emit_u16(Opcode::NewArray, 1);
                                self.emit(Opcode::ConcatArray);
                            }
                            CallArg::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::ConcatArray);
                            }
                        }
                    }
                    self.compile_expr(callee)?;
                    self.emit(Opcode::ConstructSpread);
                } else {
                    self.compile_expr(callee)?;
                    for arg in args {
                        if let CallArg::Expr(e) = arg {
                            self.compile_expr(e)?;
                        }
                    }
                    self.emit_u16(Opcode::Construct, args.len() as u16);
                }
            }
        }
        Ok(())
    }

    fn compile_jsx_element(
        &mut self,
        tag: &Arc<str>,
        props: &[JsxProp],
        children: &[JsxChild],
    ) -> Result<(), CompileError> {
        let h_idx = self.name_idx(&Arc::from("h"));
        self.emit_u16(Opcode::LoadGlobal, h_idx);
        let tag_str = tag.as_ref();
        let is_component = tag_str
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        if is_component {
            let tag_idx = self.name_idx(tag);
            self.emit_u16(Opcode::LoadGlobal, tag_idx);
        } else {
            let tag_const = self.constant_idx(Constant::String(Arc::from(tag_str)));
            self.emit(Opcode::LoadConst);
            self.chunk.write_u16(tag_const);
        }
        self.compile_jsx_props(props)?;
        self.compile_jsx_children(children)?;
        self.emit_u16(Opcode::Call, 3);
        Ok(())
    }

    fn compile_jsx_fragment(&mut self, children: &[JsxChild]) -> Result<(), CompileError> {
        let h_idx = self.name_idx(&Arc::from("h"));
        self.emit_u16(Opcode::LoadGlobal, h_idx);
        let fragment_idx = self.name_idx(&Arc::from("Fragment"));
        self.emit_u16(Opcode::LoadGlobal, fragment_idx);
        let null_idx = self.constant_idx(Constant::Null);
        self.emit(Opcode::LoadConst);
        self.chunk.write_u16(null_idx);
        self.compile_jsx_children(children)?;
        self.emit_u16(Opcode::Call, 3);
        Ok(())
    }

    fn compile_jsx_props(&mut self, props: &[JsxProp]) -> Result<(), CompileError> {
        if props.is_empty() {
            let null_idx = self.constant_idx(Constant::Null);
            self.emit(Opcode::LoadConst);
            self.chunk.write_u16(null_idx);
            return Ok(());
        }
        let has_spread = props.iter().any(|p| matches!(p, JsxProp::Spread(_)));
        if has_spread {
            self.emit_u16(Opcode::NewObject, 0);
            for prop in props {
                match prop {
                    JsxProp::Attr { name, value } => {
                        let key_idx = self.constant_idx(Constant::String(Arc::clone(name)));
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(key_idx);
                        match value {
                            JsxAttrValue::String(s) => {
                                let val_idx = self.constant_idx(Constant::String(Arc::clone(s)));
                                self.emit(Opcode::LoadConst);
                                self.chunk.write_u16(val_idx);
                            }
                            JsxAttrValue::Expr(e) => self.compile_expr(e)?,
                            JsxAttrValue::ImplicitTrue => {
                                let true_idx = self.constant_idx(Constant::Bool(true));
                                self.emit(Opcode::LoadConst);
                                self.chunk.write_u16(true_idx);
                            }
                        }
                        self.emit_u16(Opcode::NewObject, 1);
                        self.emit(Opcode::MergeObject);
                    }
                    JsxProp::Spread(expr) => {
                        self.compile_expr(expr)?;
                        self.emit(Opcode::MergeObject);
                    }
                }
            }
        } else {
            for prop in props {
                if let JsxProp::Attr { name, value } = prop {
                    let key_idx = self.constant_idx(Constant::String(Arc::clone(name)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(key_idx);
                    match value {
                        JsxAttrValue::String(s) => {
                            let val_idx = self.constant_idx(Constant::String(Arc::clone(s)));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(val_idx);
                        }
                        JsxAttrValue::Expr(e) => self.compile_expr(e)?,
                        JsxAttrValue::ImplicitTrue => {
                            let true_idx = self.constant_idx(Constant::Bool(true));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(true_idx);
                        }
                    }
                }
            }
            self.emit_u16(Opcode::NewObject, props.len() as u16);
        }
        Ok(())
    }

    fn compile_jsx_children(&mut self, children: &[JsxChild]) -> Result<(), CompileError> {
        for child in children {
            match child {
                JsxChild::Text(s) => {
                    let idx = self.constant_idx(Constant::String(Arc::clone(s)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                JsxChild::Expr(e) => self.compile_expr(e)?,
            }
        }
        self.emit_u16(Opcode::NewArray, children.len() as u16);
        Ok(())
    }
}

/// Compile a Tish program to bytecode (with peephole optimizations).
pub fn compile(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, true, false, None)
}

/// Compile, tagging the chunk with a source file path so runtime errors can report
/// `file:line` (issue #74). The line table is built during compilation and survives the
/// in-place peephole pass; it is not serialized.
pub fn compile_with_source(
    program: &Program,
    source: Option<std::sync::Arc<str>>,
) -> Result<Chunk, CompileError> {
    compile_internal(program, true, false, source)
}

/// Compile without peephole optimizations (for --no-optimize).
pub fn compile_unoptimized(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, false, false, None)
}

/// Compile for REPL: last expression statement leaves its value on the stack (no Pop, no trailing Null).
pub fn compile_for_repl(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, true, true, None)
}

/// Compile for REPL without peephole optimizations.
pub fn compile_for_repl_unoptimized(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, false, true, None)
}

fn compile_internal(
    program: &Program,
    peephole: bool,
    retain_last_expr: bool,
    source: Option<std::sync::Arc<str>>,
) -> Result<Chunk, CompileError> {
    let mut chunk = Chunk::new();
    chunk.source = source; // tag before compiling so nested chunks inherit it (#74)
    let mut compiler = Compiler::new(&mut chunk, retain_last_expr);
    // #186 — `Math` intrinsics are sound only if `Math` is never rebound anywhere in the program.
    // `stmt_rebinds` is conservative (any rebind, destructure, FunDecl, or unknown node → true), so a
    // `false` here only forgoes the optimization, never risks a miscompile.
    // #203: whole-program scan (recurses into function bodies/params, unlike `stmt_rebinds` which is
    // conservatively `true` on any FunDecl) — so `Math` is provably-global even when the program
    // defines functions, letting `Math.<fn>` inside a function lower to the MathUnary intrinsic (the
    // numeric/array JIT then compiles the kernel instead of bailing on a generic call). Errs toward
    // "rebound" ⇒ never falsely global ⇒ no miscompile. Threaded into nested compilers below.
    compiler.math_is_global = !program
        .statements
        .iter()
        .any(|s| name_rebinds_in_stmt(s, "Math"));
    // #187 — the set of top-level functions safe to call directly from JIT'd code (never reassigned/
    // shadowed/redeclared anywhere). Conservative: a name absent here only forgoes the optimization.
    compiler.stable_globals = Arc::new(compute_stable_globals(program));
    compiler.compile_program(program)?;
    if peephole {
        crate::peephole::optimize(&mut chunk);
    }
    Ok(chunk)
}

#[cfg(test)]
mod stable_globals_tests {
    use super::compute_stable_globals;

    fn stable(src: &str) -> std::collections::HashSet<String> {
        let prog = tishlang_parser::parse(src).expect("parse");
        compute_stable_globals(&prog)
            .into_iter()
            .map(|n| n.to_string())
            .collect()
    }

    /// #187 gate: the exact spectral_norm shape — five top-level functions, none reassigned — must ALL
    /// be provably stable (this is what lets `multiplyAv` directly call `evalA`). The naive
    /// `stmt_rebinds`-based scan would return the empty set here (it treats every `FunDecl` as a
    /// rebind), so this is the regression tripwire for the purpose-built analysis.
    #[test]
    fn all_unreassigned_top_level_fns_are_stable() {
        let s = stable(
            "function evalA(i, j) { return 1.0 / (i + j) }\n\
             function multiplyAv(n, v, av) { let i = 0; while (i < n) { av[i] = evalA(i, i) * v[i]; i = i + 1 } }\n\
             function spectralNorm(n) { multiplyAv(n, n, n); return n }\n",
        );
        assert!(
            s.contains("evalA"),
            "evalA (called by multiplyAv) must be stable"
        );
        assert!(s.contains("multiplyAv"));
        assert!(s.contains("spectralNorm"));
    }

    /// A reassigned function is NOT stable (a direct call could hit a stale binding) — but a sibling
    /// that is never reassigned still is.
    #[test]
    fn reassigned_function_is_excluded() {
        let s = stable(
            "function f(x) { return x + 1 }\n\
             function g(x) { return f(x) * 2 }\n\
             f = (x) => x + 100\n",
        );
        assert!(!s.contains("f"), "f is reassigned → must NOT be stable");
        assert!(s.contains("g"), "g is never reassigned → stable");
    }

    /// Reassignment/redeclaration/shadowing INSIDE another function body must disqualify the name —
    /// the whole-program walk has to recurse into every body (the whole point vs `stmt_rebinds`).
    #[test]
    fn cross_body_rebind_and_shadow_and_redecl_excluded() {
        // reassigned inside another function's body
        assert!(!stable(
            "function h(x) { return x }\n\
             function k() { h = 5; return h }\n"
        )
        .contains("h"));
        // shadowed by a param in another function
        assert!(!stable(
            "function p(x) { return x }\n\
             function q(p) { return p }\n"
        )
        .contains("p"));
        // declared twice at top level
        assert!(!stable("function d(x) { return x }\nfunction d(y) { return y }\n").contains("d"));
        // also bound by a top-level let
        assert!(!stable("function e(x) { return x }\nlet e = 3\n").contains("e"));
    }

    /// A global reassigned inside a PARAMETER DEFAULT (of a function or arrow) must disqualify it — the
    /// default runs in the enclosing scope when the function is called, so a direct call would be stale.
    #[test]
    fn param_default_rebind_excluded() {
        // `foo` reassigned in another function's param default
        assert!(!stable(
            "function evil(x) { return x + 1000 }\n\
             function foo(x) { return x + 1 }\n\
             function resetFoo(y = (foo = evil)) { return y }\n"
        )
        .contains("foo"));
        // `bar` reassigned in an arrow's param default
        assert!(!stable(
            "function bar(x) { return x }\n\
             let f = (z = (bar = 5)) => z\n"
        )
        .contains("bar"));
        // a param default that does NOT touch the name leaves it stable
        assert!(
            stable("function ok(x) { return x }\nfunction u(a = 1) { return a }\n").contains("ok")
        );
    }
}
