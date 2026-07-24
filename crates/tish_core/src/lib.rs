//! Tish Core - Shared types and utilities for the Tish language.
//!
//! This crate provides the unified Value type and utilities used by both
//! the interpreter (tishlang_eval) and compiled runtime (tishlang_runtime).
#![cfg_attr(feature = "portable", no_std)]

// `portable` (no_std, Arc→Rc) and `send-values` (parking_lot, std-only) are mutually
// exclusive; unified by Cargo's additive features they'd try to compile parking_lot for a
// no_std target with a confusing far-from-root-cause error. Fail loudly at the source instead.
#[cfg(all(feature = "portable", feature = "send-values"))]
compile_error!("tishlang_core features `portable` and `send-values` are mutually exclusive");

// `portable` implies `#![no_std]`, so `std` must be OFF (depend on this crate as
// `default-features = false, features = ["portable"]`). If both are on — the usual slip of
// requesting `portable` without disabling the default `std` feature — the `no_std` attribute
// above still applies while the `std`-only deps (ahash/arcstr) are pulled in, producing a
// confusing cascade. Catch it at the source with a message that names the fix.
#[cfg(all(feature = "portable", feature = "std"))]
compile_error!(
    "tishlang_core feature `portable` requires `default-features = false` (the default `std` feature must be disabled)"
);

extern crate alloc;

#[cfg(feature = "portable")]
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

mod compat;
mod console_style;
mod json;
mod macros;
mod shape;
mod uri;
mod value;
mod vmref;

/// Hidden re-export so the exported `tish_module!` macro can name the right
/// smart pointer (`std::sync::Arc` on host, `Rc` under `portable`) at its
/// expansion sites without leaking the private `compat` module.
#[doc(hidden)]
pub use compat::Arc as __TishArc;

/// Portable-runtime surface consumed by `tishlang_builtins` and the GBA facade
/// (`tishlang_runtime_gba`): the libm-backed float trait and the installable
/// clock / RNG hooks. Only present under `portable`.
#[cfg(feature = "portable")]
pub use compat::{
    install_clock, install_rng, next_u64, now_ms, random_f64, seed_rng, FloatExt, SingleCore,
};

pub use console_style::{format_value_styled, format_values_for_console, use_console_colors};
pub use json::{
    escape_json_string_into, json_parse, json_stringify, json_stringify_into, write_json_number,
};
pub use shape::{ShapeId, DICT_SHAPE, EMPTY_SHAPE};
pub use uri::{percent_decode, percent_decode_component, percent_encode, percent_encode_component};
pub use compat::ArcStr;
/// The core type vocabulary, re-exported so `tishlang_builtins`, generated code,
/// and the GBA facade share one source of truth: `Arc` (std `Arc` / `Rc` under
/// `portable`), and the object hasher (`ahash` / `FxHasher`). Names match the std
/// originals so hosted call sites are unchanged.
pub use compat::{AHashMap, Arc, RandomState};
/// Lock / once-cell vocabulary, namespaced to avoid polluting the crate root:
/// std `sync` types on the host, single-core shims under `portable`. Used by
/// `tishlang_builtins` for its symbol registry.
pub mod sync {
    pub use crate::compat::{Mutex, OnceLock, RwLock};
}
pub use value::*;
pub use vmref::{VmReadGuard, VmRef, VmWriteGuard};

/// `process.argv` for the interpreter / VM. Defaults to the host process's own `std::env::args()`,
/// but `tish run <file> [args...]` overrides it (via [`set_process_argv`]) with a node-shaped argv
/// `[tish-exe, <file>, args...]` so a script sees its own args — not the `run` subcommand. Compiled
/// native binaries don't touch this; they read `std::env::args()` directly (which is already their
/// own argv). #88
#[cfg(not(feature = "portable"))]
static PROCESS_ARGV: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Override `process.argv` for the interpreter/VM (set once by `tish run` before executing). A
/// later call is ignored (the first override wins), matching the single-program-per-process model.
/// No-op under `portable` (an embedded GBA ROM has no process argv).
#[cfg(not(feature = "portable"))]
pub fn set_process_argv(argv: Vec<String>) {
    let _ = PROCESS_ARGV.set(argv);
}

/// See [`set_process_argv`]. No-op under `portable`.
#[cfg(feature = "portable")]
pub fn set_process_argv(_argv: Vec<String>) {}

/// The argv a script should see as `process.argv`: the [`set_process_argv`] override if present,
/// else the host process's `std::env::args()`. Empty under `portable`.
pub fn process_argv() -> Vec<String> {
    #[cfg(not(feature = "portable"))]
    {
        match PROCESS_ARGV.get() {
            Some(v) => v.clone(),
            None => std::env::args().collect(),
        }
    }
    #[cfg(feature = "portable")]
    {
        Vec::new()
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
#[cfg(not(feature = "portable"))]
thread_local! {
    static PENDING_THROW: core::cell::RefCell<Option<Value>> = const { core::cell::RefCell::new(None) };
}
// Single-core: a plain static with the same `.with()` API. See compat::SingleCore.
#[cfg(feature = "portable")]
static PENDING_THROW: compat::SingleCore<core::cell::RefCell<Option<Value>>> =
    compat::SingleCore::new(core::cell::RefCell::new(None));

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

// Current user-function call depth for the bytecode VM. The VM's recursive path builds a fresh
// `Vm` per call (so no shared struct field can accumulate) and its `Callable::call` signature is
// fixed — a thread-local is the VM's equivalent of the interpreter's shared `Rc<Cell>` counter.
// Lives here (not `tish_vm`) beside `PENDING_THROW` for the same layering reason. #381
#[cfg(not(feature = "portable"))]
thread_local! {
    static CALL_DEPTH: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}
#[cfg(feature = "portable")]
static CALL_DEPTH: compat::SingleCore<core::cell::Cell<usize>> =
    compat::SingleCore::new(core::cell::Cell::new(0));

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
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_MAX_CALL_DEPTH: usize = 20_000;

/// wasm32 ceiling (#531). On wasm the embedded VM runs on the single guest stack — there is no host
/// worker thread to grow into (`Vm` can't spawn on wasm), and the host runtime (wasmtime) bounds
/// guest call depth by its own `max_wasm_stack`. That limit is NOT reachable from the `.wasm`: it
/// overflows into an UNCATCHABLE module trap after only ~300 `Vm::run_chunk` frames — the tish
/// function locals live in the VM operand stack, so the per-frame host cost is ~constant regardless
/// of the program, and the trap depth is a stable ~310 under default wasmtime. A far lower ceiling
/// keeps the frame-counting guard firing first (measured safe with margin below the trap), so deep
/// recursion raises a CATCHABLE `RangeError` on WASI too — matching interp/vm/native rather than
/// aborting the process. WASI programs are thus capped much shallower than native; that is inherent
/// to wasm's host-bounded call stack, not a tish choice.
#[cfg(target_arch = "wasm32")]
pub const DEFAULT_MAX_CALL_DEPTH: usize = 256;

// The recursion ceiling, lazily initialized from `TISH_MAX_CALL_DEPTH` (0 = uninitialized). A
// thread-local Cell (not OnceLock) so tests can override it per-thread without racing the
// process-wide env. Shared by the VM's guard and the native backend's boxed-call guard. #381
#[cfg(not(feature = "portable"))]
thread_local! {
    static MAX_CALL_DEPTH: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}
#[cfg(feature = "portable")]
static MAX_CALL_DEPTH: compat::SingleCore<core::cell::Cell<usize>> =
    compat::SingleCore::new(core::cell::Cell::new(0));

/// The recursion ceiling for depth-counting guards: user-fn calls deeper than this raise a catchable
/// `RangeError` instead of overflowing the native stack. `TISH_MAX_CALL_DEPTH` overrides (default
/// [`DEFAULT_MAX_CALL_DEPTH`]). #381
pub fn max_call_depth() -> usize {
    MAX_CALL_DEPTH.with(|c| {
        let v = c.get();
        if v != 0 {
            return v;
        }
        #[cfg(not(feature = "portable"))]
        let init = std::env::var("TISH_MAX_CALL_DEPTH")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_MAX_CALL_DEPTH);
        #[cfg(feature = "portable")]
        let init = DEFAULT_MAX_CALL_DEPTH;
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
        Arc::from("name"),
        Value::String("RangeError".into()),
    );
    e.insert(
        Arc::from("message"),
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
        Arc::from("name"),
        Value::String("TypeError".into()),
    );
    e.insert(
        Arc::from("message"),
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
        Arc::from("name"),
        Value::String("TypeError".into()),
    );
    e.insert(
        Arc::from("message"),
        Value::String(message.into().into()),
    );
    Value::object(e)
}

/// A string key names a canonical array index iff it is a non-negative integer whose decimal form
/// round-trips and is below the array-index ceiling (`2³²−1`): `"0"`, `"12"` → `Some`; `"01"`, `"-1"`,
/// `"1.5"`, `"foo"`, `" 1"`, `""` → `None`. Shared so `arr["0"] === arr[0]` on every backend (#432).
pub fn str_to_array_index(s: &str) -> Option<usize> {
    let i: usize = s.parse().ok()?;
    if i < 4_294_967_295 && i.to_string() == s {
        Some(i)
    } else {
        None
    }
}

/// A catchable `RangeError` with an arbitrary message (`{ name: "RangeError", message }`), matching the
/// VM/interpreter/node shape. Used to PARK a pending throw from a shared builtin that returns a plain
/// `Value` (e.g. `[1,2].with(5, x)` with an out-of-range index).
pub fn range_error(message: impl Into<String>) -> Value {
    let mut e = ObjectMap::default();
    e.insert(
        Arc::from("name"),
        Value::String("RangeError".into()),
    );
    e.insert(
        Arc::from("message"),
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
        Arc::from("name"),
        Value::String("TypeError".into()),
    );
    e.insert(
        Arc::from("message"),
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
