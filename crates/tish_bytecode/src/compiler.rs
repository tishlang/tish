//! AST to bytecode compiler.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, DestructElement, DestructPattern, ExportDeclaration,
    Expr, FunParam, JsxAttrValue, JsxChild, JsxProp, Literal, LogicalAssignOp, MemberProp,
    ObjectProp, Program, Span, Statement,
};

use crate::chunk::{Chunk, Constant};
use crate::encoding::{binop_to_u8, compound_op_to_u8, unaryop_to_u8};
use crate::opcode::Opcode;

enum SimpleMapResult {
    Identity,
    BinOp(BinOp, Constant, bool), // op, constant, param_on_left
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
    finally_stack: Vec<Box<Statement>>,
    /// When `Some(name)`, this chunk is the body of `fn name(...)` and `name`'s binding is provably
    /// stable (no param shadows it, no reassignment/redeclaration in the body — see [`stmt_rebinds`]).
    /// A direct call `name(args)` then compiles to `SelfCall` (no name lookup / closure dispatch; the
    /// JIT lowers it to a native recursive call). `None` for anonymous fns, top-level, or anywhere the
    /// self-binding can't be proven stable.
    self_fn_name: Option<Arc<str>>,
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
            ObjectProp::KeyValue(_, x) => expr_is_param_only(x, params),
            ObjectProp::Spread(_) => false,
        }),
        Expr::TemplateLiteral { exprs, .. } => {
            exprs.iter().all(|x| expr_is_param_only(x, params))
        }
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
        Statement::ExprStmt { expr, .. } => expr_is_param_only(expr, params),
        Statement::Return { value, .. } => {
            value.as_ref().map_or(true, |e| expr_is_param_only(e, params))
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
                    .map_or(true, |b| stmt_is_param_only(b, params))
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
    std::env::var("TISH_VM_SLOTS").map(|v| v != "0").unwrap_or(true)
}

/// Is `name` bound by one of `params` (so it would shadow a function's own name)? Conservative:
/// any destructuring param returns `true` (it could bind `name` via a nested pattern we don't analyze).
fn params_bind_name(params: &[FunParam], name: &str) -> bool {
    params.iter().any(|p| match p {
        FunParam::Simple(tp) => tp.name.as_ref() == name,
        FunParam::Destructure { .. } => true,
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
        Statement::VarDecl { name: n, init, .. } => {
            n.as_ref() == name || init.as_ref().is_some_and(|e| expr_rebinds(e, name))
        }
        Statement::ExprStmt { expr, .. } => expr_rebinds(expr, name),
        Statement::Return { value, .. } => value.as_ref().is_some_and(|e| expr_rebinds(e, name)),
        Statement::Throw { value, .. } => expr_rebinds(value, name),
        Statement::If { cond, then_branch, else_branch, .. } => {
            expr_rebinds(cond, name)
                || stmt_rebinds(then_branch, name)
                || else_branch.as_ref().is_some_and(|s| stmt_rebinds(s, name))
        }
        Statement::While { cond, body, .. } => expr_rebinds(cond, name) || stmt_rebinds(body, name),
        Statement::DoWhile { body, cond, .. } => stmt_rebinds(body, name) || expr_rebinds(cond, name),
        Statement::For { init, cond, update, body, .. } => {
            init.as_ref().is_some_and(|s| stmt_rebinds(s, name))
                || cond.as_ref().is_some_and(|e| expr_rebinds(e, name))
                || update.as_ref().is_some_and(|e| expr_rebinds(e, name))
                || stmt_rebinds(body, name)
        }
        Statement::ForOf { name: n, iterable, body, .. } => {
            n.as_ref() == name || expr_rebinds(iterable, name) || stmt_rebinds(body, name)
        }
        Statement::Switch { expr, cases, default_body, .. } => {
            expr_rebinds(expr, name)
                || cases.iter().any(|(t, body)| {
                    t.as_ref().is_some_and(|e| expr_rebinds(e, name))
                        || body.iter().any(|s| stmt_rebinds(s, name))
                })
                || default_body
                    .as_ref()
                    .is_some_and(|b| b.iter().any(|s| stmt_rebinds(s, name)))
        }
        Statement::Try { body, catch_body, finally_body, .. } => {
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
        | Expr::LogicalAssign { name: n, value, .. } => n.as_ref() == name || expr_rebinds(value, name),
        Expr::PostfixInc { name: n, .. }
        | Expr::PostfixDec { name: n, .. }
        | Expr::PrefixInc { name: n, .. }
        | Expr::PrefixDec { name: n, .. } => n.as_ref() == name,
        Expr::Literal { .. } | Expr::Ident { .. } => false,
        Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
            expr_rebinds(left, name) || expr_rebinds(right, name)
        }
        Expr::Unary { operand, .. } | Expr::TypeOf { operand, .. } | Expr::Await { operand, .. } => {
            expr_rebinds(operand, name)
        }
        Expr::Conditional { cond, then_branch, else_branch, .. } => {
            expr_rebinds(cond, name) || expr_rebinds(then_branch, name) || expr_rebinds(else_branch, name)
        }
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            expr_rebinds(callee, name)
                || args.iter().any(|a| match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => expr_rebinds(e, name),
                })
        }
        Expr::Member { object, .. } => expr_rebinds(object, name),
        Expr::Index { object, index, .. } => expr_rebinds(object, name) || expr_rebinds(index, name),
        Expr::Array { elements, .. } => elements.iter().any(|el| match el {
            ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_rebinds(e, name),
        }),
        Expr::Object { props, .. } => props.iter().any(|p| match p {
            ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => expr_rebinds(e, name),
        }),
        Expr::MemberAssign { object, value, .. } => expr_rebinds(object, name) || expr_rebinds(value, name),
        Expr::IndexAssign { object, index, value, .. } => {
            expr_rebinds(object, name) || expr_rebinds(index, name) || expr_rebinds(value, name)
        }
        Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(|e| expr_rebinds(e, name)),
        // A nested closure could reassign the outer `name`; recurse (over-conservative if it shadows,
        // which only costs the optimization).
        Expr::ArrowFunction { body, .. } => match body {
            ArrowBody::Expr(e) => expr_rebinds(e, name),
            ArrowBody::Block(s) => stmt_rebinds(s, name),
        },
        // Jsx, NativeModuleLoad, and anything unknown → conservative.
        _ => true,
    }
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
            Statement::Block { statements, .. } => statements.iter().all(|s| self.stmt(s, in_closure)),
            Statement::VarDecl { init, .. } => init.as_ref().map_or(true, |e| self.expr(e, in_closure)),
            Statement::VarDeclDestructure { init, .. } => self.expr(init, in_closure),
            Statement::ExprStmt { expr, .. } => self.expr(expr, in_closure),
            Statement::If { cond, then_branch, else_branch, .. } => {
                self.expr(cond, in_closure)
                    && self.stmt(then_branch, in_closure)
                    && else_branch.as_ref().map_or(true, |s| self.stmt(s, in_closure))
            }
            Statement::While { cond, body, .. } => self.expr(cond, in_closure) && self.stmt(body, in_closure),
            Statement::DoWhile { body, cond, .. } => self.stmt(body, in_closure) && self.expr(cond, in_closure),
            Statement::For { init, cond, update, body, .. } => {
                init.as_ref().map_or(true, |i| self.stmt(i, in_closure))
                    && cond.as_ref().map_or(true, |e| self.expr(e, in_closure))
                    && update.as_ref().map_or(true, |e| self.expr(e, in_closure))
                    && self.stmt(body, in_closure)
            }
            Statement::ForOf { iterable, body, .. } => {
                self.expr(iterable, in_closure) && self.stmt(body, in_closure)
            }
            Statement::Return { value, .. } => value.as_ref().map_or(true, |e| self.expr(e, in_closure)),
            Statement::Throw { value, .. } => self.expr(value, in_closure),
            Statement::Break { .. } | Statement::Continue { .. } => true,
            Statement::Switch { expr, cases, default_body, .. } => {
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
                    .map_or(true, |b| b.iter().all(|s| self.stmt(s, in_closure)))
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                self.stmt(body, in_closure)
                    && catch_body.as_ref().map_or(true, |s| self.stmt(s, in_closure))
                    && finally_body.as_ref().map_or(true, |s| self.stmt(s, in_closure))
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
            Expr::Binary { left, right, .. } => self.expr(left, in_closure) && self.expr(right, in_closure),
            Expr::Unary { operand, .. } | Expr::TypeOf { operand, .. } | Expr::Await { operand, .. } => {
                self.expr(operand, in_closure)
            }
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                self.expr(cond, in_closure) && self.expr(then_branch, in_closure) && self.expr(else_branch, in_closure)
            }
            Expr::NullishCoalesce { left, right, .. } => self.expr(left, in_closure) && self.expr(right, in_closure),
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                self.expr(callee, in_closure)
                    && args.iter().all(|a| match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => self.expr(e, in_closure),
                    })
            }
            Expr::Member { object, .. } => self.expr(object, in_closure),
            Expr::Index { object, index, .. } => self.expr(object, in_closure) && self.expr(index, in_closure),
            Expr::Array { elements, .. } => elements.iter().all(|el| match el {
                ArrayElement::Expr(e) | ArrayElement::Spread(e) => self.expr(e, in_closure),
            }),
            Expr::Object { props, .. } => props.iter().all(|p| match p {
                ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => self.expr(e, in_closure),
            }),
            Expr::Assign { name, value, .. }
            | Expr::CompoundAssign { name, value, .. }
            | Expr::LogicalAssign { name, value, .. } => {
                if in_closure {
                    self.captured.insert(Arc::clone(name));
                }
                self.expr(value, in_closure)
            }
            Expr::MemberAssign { object, value, .. } => self.expr(object, in_closure) && self.expr(value, in_closure),
            Expr::IndexAssign { object, index, value, .. } => {
                self.expr(object, in_closure) && self.expr(index, in_closure) && self.expr(value, in_closure)
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
fn slot_analyze(params: &[FunParam], has_rest: bool, body: &Statement) -> Option<HashSet<Arc<str>>> {
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
        self.slot_scopes.iter().rev().find_map(|m| m.get(name).copied())
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
                self.compile_expr(expr)?;
                self.emit(Opcode::Pop);
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.compile_expr(cond)?;
                let jump_else = self.emit_jump(Opcode::JumpIfFalse);
                self.compile_statement(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_else, self.chunk.code.len());
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
                self.compile_expr(cond)?;
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                // JumpIfFalse already pops condition when taking body
                self.compile_statement(body)?;
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_out, end);
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
                if let Some(c) = cond {
                    self.compile_expr(c)?;
                } else {
                    let idx = self.constant_idx(Constant::Bool(true));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
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
                self.patch_jump(jump_out, end);
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
                let loop_start = self.chunk.code.len();
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                    continue_is_forward_jump: false,
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
                self.emit_u16(Opcode::LoadVar, i_idx);
                let one_idx = self.constant_idx(Constant::Number(1.0));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one_idx);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit_u16(Opcode::StoreVar, i_idx);
                self.emit_u16(Opcode::LoadVar, i_idx);
                self.emit_u16(Opcode::LoadVar, len_idx);
                self.emit_u8(Opcode::BinOp, 10);
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(loop_start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_out, end);
                let info = self.loop_stack.pop().unwrap();
                self.breakable_stack.pop();
                for p in info.continue_patches {
                    self.patch_jump_back(p, loop_start);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                self.emit(Opcode::LoopVarsEnd);
                self.exit_block_scope();
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
                let mut inner_comp = Compiler::new(&mut inner, false);
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
                    inner_comp.emit_param_destructure_prologue(&param_names[..formal_len], &slots)?;
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
                    continue_is_forward_jump: false,
                });
                self.breakable_stack.push(Breakable::Loop {
                    unwind_depth: self.block_depth,
                });
                self.compile_statement(body)?;
                let cond_start = self.chunk.code.len();
                self.compile_expr(cond)?;
                let jump_back = self.emit_jump(Opcode::JumpIfFalse);
                let jump_back_dist = (self.chunk.code.len() + 3).saturating_sub(start);
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_back, end);
                let info = self.loop_stack.pop().unwrap();
                self.breakable_stack.pop();
                for p in info.continue_patches {
                    self.patch_jump_back(p, cond_start);
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
                    self.finally_stack.push(f.clone());
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
                        // Short-circuit: a && b => if !a then a else b
                        self.compile_expr(left)?;
                        self.emit(Opcode::Dup);
                        let jump_shortcut = self.emit_jump(Opcode::JumpIfFalse);
                        self.compile_expr(right)?; // left still on stack from Dup
                        self.emit_u8(Opcode::BinOp, binop_to_u8(BinOp::And));
                        let jump_end = self.emit_jump(Opcode::Jump);
                        self.patch_jump(jump_shortcut, self.chunk.code.len());
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
                            ObjectProp::KeyValue(k, v) => {
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
                        if let ObjectProp::KeyValue(k, v) = prop {
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
                for p in &param_names {
                    inner.add_name(Arc::clone(p));
                }
                inner.param_count = param_names.len() as u16;
                if simple_slots.is_some() {
                    inner.slot_based = true;
                    inner.num_slots = param_names.len() as u16;
                }
                let mut inner_comp = Compiler::new(&mut inner, false);
                if let Some(map) = simple_slots {
                    inner_comp.slot_ctx = Some(map);
                } else {
                    inner_comp.scope = vec![param_names
                        .iter()
                        .map(|n| (Arc::clone(n), false))
                        .collect::<HashMap<_, _>>()];
                    inner_comp.emit_param_destructure_prologue(&param_names[..formal_len], &slots)?;
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
    compile_internal(program, true, false)
}

/// Compile without peephole optimizations (for --no-optimize).
pub fn compile_unoptimized(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, false, false)
}

/// Compile for REPL: last expression statement leaves its value on the stack (no Pop, no trailing Null).
pub fn compile_for_repl(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, true, true)
}

/// Compile for REPL without peephole optimizations.
pub fn compile_for_repl_unoptimized(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, false, true)
}

fn compile_internal(
    program: &Program,
    peephole: bool,
    retain_last_expr: bool,
) -> Result<Chunk, CompileError> {
    let mut chunk = Chunk::new();
    let mut compiler = Compiler::new(&mut chunk, retain_last_expr);
    compiler.compile_program(program)?;
    if peephole {
        crate::peephole::optimize(&mut chunk);
    }
    Ok(chunk)
}
