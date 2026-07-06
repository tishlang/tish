//! Tish Core - Shared types and utilities for the Tish language.
//!
//! This crate provides the unified Value type and utilities used by both
//! the interpreter (tishlang_eval) and compiled runtime (tishlang_runtime).

mod console_style;
mod json;
mod macros;
mod shape;
mod uri;
mod value;
mod vmref;

pub use console_style::{format_value_styled, format_values_for_console, use_console_colors};
pub use json::{
    escape_json_string_into, json_parse, json_stringify, json_stringify_into, write_json_number,
};
pub use shape::{ShapeId, DICT_SHAPE, EMPTY_SHAPE};
pub use uri::{percent_decode, percent_decode_component, percent_encode, percent_encode_component};
pub use arcstr::ArcStr;
pub use value::*;
pub use vmref::{VmReadGuard, VmRef, VmWriteGuard};

/// `process.argv` for the interpreter / VM. Defaults to the host process's own `std::env::args()`,
/// but `tish run <file> [args...]` overrides it (via [`set_process_argv`]) with a node-shaped argv
/// `[tish-exe, <file>, args...]` so a script sees its own args — not the `run` subcommand. Compiled
/// native binaries don't touch this; they read `std::env::args()` directly (which is already their
/// own argv). #88
static PROCESS_ARGV: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Override `process.argv` for the interpreter/VM (set once by `tish run` before executing). A
/// later call is ignored (the first override wins), matching the single-program-per-process model.
pub fn set_process_argv(argv: Vec<String>) {
    let _ = PROCESS_ARGV.set(argv);
}

/// The argv a script should see as `process.argv`: the [`set_process_argv`] override if present,
/// else the host process's `std::env::args()`.
pub fn process_argv() -> Vec<String> {
    match PROCESS_ARGV.get() {
        Some(v) => v.clone(),
        None => std::env::args().collect(),
    }
}

/// #303 — a thrown JS value parked while it unwinds across a boundary that can't carry a `Result`.
///
/// The native value-fn ABI is `Fn(&[Value]) -> Value`, and `Callable::call` likewise returns a bare
/// `Value`, so a `throw` crossing such a boundary can't ride a `Result`. It is parked here and picked
/// up at the next checkpoint: native codegen checks it after each call; the VM checks it after each
/// `Callable::call`; and the shared array builtins (`forEach`/`map`/`sort`/…) check it between
/// elements so they stop iterating promptly instead of running the callback for the rest of the
/// array. The slot lives in `tishlang_core` (rather than `tishlang_runtime`) so `tishlang_builtins`
/// can poll it without a `builtins -> runtime` dependency cycle; `tishlang_runtime` and `tishlang_vm`
/// share this one slot. First-throw-wins; drained exactly once by [`take_pending_throw`] at the frame
/// that converts it back into a `Result`.
thread_local! {
    static PENDING_THROW: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// Park a thrown value to propagate across a non-`Result` boundary. First-throw-wins: if one is
/// already pending (an erroneous continuation reached a second throw before the slot was drained),
/// keep the first — that is the throw JS would have raised.
pub fn set_pending_throw(v: Value) {
    PENDING_THROW.with(|c| {
        let mut slot = c.borrow_mut();
        if slot.is_none() {
            *slot = Some(v);
        }
    });
}

/// Is a thrown value waiting to propagate?
pub fn has_pending_throw() -> bool {
    PENDING_THROW.with(|c| c.borrow().is_some())
}

/// Take the parked thrown value, clearing the slot (drains it exactly once).
pub fn take_pending_throw() -> Option<Value> {
    PENDING_THROW.with(|c| c.borrow_mut().take())
}

thread_local! {
    /// Current user-function call depth for the bytecode VM. The VM's recursive path builds a fresh
    /// `Vm` per call (so no shared struct field can accumulate) and its `Callable::call` signature is
    /// fixed — a thread-local is the VM's equivalent of the interpreter's shared `Rc<Cell>` counter.
    /// Lives here (not `tish_vm`) beside `PENDING_THROW` for the same layering reason. #381
    static CALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Enter one VM call frame; returns the new depth. Pair every call with [`dec_call_depth`].
#[inline]
pub fn inc_call_depth() -> usize {
    CALL_DEPTH.with(|c| {
        let d = c.get() + 1;
        c.set(d);
        d
    })
}

/// Leave one VM call frame.
#[inline]
pub fn dec_call_depth() {
    CALL_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
}

/// Default recursion ceiling shared by every backend that counts call depth (#381). Chosen with the
/// interpreter: comfortably deep for real programs, but reached long before counted recursion can
/// exhaust memory or the stack.
pub const DEFAULT_MAX_CALL_DEPTH: usize = 20_000;

thread_local! {
    // The recursion ceiling, lazily initialized from `TISH_MAX_CALL_DEPTH` (0 = uninitialized). A
    // thread-local Cell (not OnceLock) so tests can override it per-thread without racing the
    // process-wide env. Shared by the VM's guard and the native backend's boxed-call guard. #381
    static MAX_CALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The recursion ceiling for depth-counting guards: user-fn calls deeper than this raise a catchable
/// `RangeError` instead of overflowing the native stack. `TISH_MAX_CALL_DEPTH` overrides (default
/// [`DEFAULT_MAX_CALL_DEPTH`]). #381
pub fn max_call_depth() -> usize {
    MAX_CALL_DEPTH.with(|c| {
        let v = c.get();
        if v != 0 {
            return v;
        }
        let init = std::env::var("TISH_MAX_CALL_DEPTH")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_MAX_CALL_DEPTH);
        c.set(init);
        init
    })
}

/// Test hook: pin the ceiling for the current thread (bypasses the env read).
pub fn set_max_call_depth_for_test(n: usize) {
    MAX_CALL_DEPTH.with(|c| c.set(n));
}

/// The catchable `RangeError` raised when a recursion guard trips. Built by hand (`tishlang_core`
/// sits below `tishlang_builtins`) but shape-identical to `construct_builtin::error_object`:
/// a `{ name, message }` object — keep the two in lock-step.
pub fn stack_overflow_error() -> Value {
    let mut e = ObjectMap::default();
    e.insert(
        std::sync::Arc::from("name"),
        Value::String("RangeError".into()),
    );
    e.insert(
        std::sync::Arc::from("message"),
        Value::String("Maximum call stack size exceeded".into()),
    );
    Value::object(e)
}

/// A catchable `TypeError` for calling a non-callable value — the `{ name, message }` shape matches
/// the VM/interpreter/node (`e.name === "TypeError"`). Used to PARK a pending throw (#381) instead of
/// aborting the process, so `value_call` on a non-function surfaces as a catchable error at the next
/// pending-throw checkpoint rather than an uncatchable native panic.
pub fn not_a_function_error(message: impl Into<String>) -> Value {
    let mut e = ObjectMap::default();
    e.insert(
        std::sync::Arc::from("name"),
        Value::String("TypeError".into()),
    );
    e.insert(
        std::sync::Arc::from("message"),
        Value::String(message.into().into()),
    );
    Value::object(e)
}

/// A catchable `TypeError` with an arbitrary message (`{ name: "TypeError", message }`), matching the
/// VM/interpreter/node shape. Used to PARK a pending throw from a shared builtin that returns a plain
/// `Value` and so cannot signal an error with `Result` (e.g. `[].reduce(fn)` with no initial value).
pub fn type_error(message: impl Into<String>) -> Value {
    let mut e = ObjectMap::default();
    e.insert(
        std::sync::Arc::from("name"),
        Value::String("TypeError".into()),
    );
    e.insert(
        std::sync::Arc::from("message"),
        Value::String(message.into().into()),
    );
    Value::object(e)
}

/// A catchable `TypeError` for reading a property/index of the nullish value — the `{ name, message }`
/// shape matches the VM/interpreter/node (`e.name === "TypeError"`). Used to PARK a pending throw in
/// the native/runtime `get_prop`/`get_index` null arm (#425), so `null.length` / `null[0]` surface a
/// catchable error at the next pending-throw checkpoint instead of silently reading back `null`.
pub fn cannot_read_property_error(prop: &str) -> Value {
    let mut e = ObjectMap::default();
    e.insert(
        std::sync::Arc::from("name"),
        Value::String("TypeError".into()),
    );
    e.insert(
        std::sync::Arc::from("message"),
        Value::String(format!("Cannot read property '{}' of null", prop).into()),
    );
    Value::object(e)
}

/// RAII frame for the depth-counting recursion guard: holding one means the depth was incremented;
/// dropping it decrements, so early returns in generated code can't leak a level. #381
pub struct CallDepthGuard(());

impl Drop for CallDepthGuard {
    #[inline]
    fn drop(&mut self) {
        dec_call_depth();
    }
}

/// Enter a counted user-fn call frame, or trip the recursion guard: past [`max_call_depth`] this
/// parks the catchable stack-overflow `RangeError` in the pending-throw slot and returns `None` —
/// the caller just returns its frame's dummy value (`Value::Null`) and the throw surfaces at the
/// next pending-throw checkpoint. The native backend emits this at the top of every boxed user-fn
/// closure. #381
#[inline]
pub fn enter_call_guarded() -> Option<CallDepthGuard> {
    let depth = inc_call_depth();
    if depth > max_call_depth() {
        dec_call_depth();
        set_pending_throw(stack_overflow_error());
        return None;
    }
    Some(CallDepthGuard(()))
}
