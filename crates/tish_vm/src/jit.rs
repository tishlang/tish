//! Numeric JIT — native codegen for `slot_based` numeric functions.
//!
//! Compiles numeric `f64`-in/`f64`-out functions to native machine code via Cranelift; the VM calls
//! them directly from the `Call` path when every argument is a number. Two builders:
//!   * [`build_body`] — straight-line + the ternary-`select` shape (leaf callbacks `x => x * 2`,
//!     `(a, b) => a - b`, `x => x === 500`, …).
//!   * [`build_body_cfg`] — **functions with LOOPS and branches** (the big win: a numeric loop
//!     function went from ~89× Node interpreted to ≈1× Node native). Uses a cranelift `Variable` per
//!     frame slot and a block per bytecode jump target; handles for/while/nested loops, if/else,
//!     early return, break, continue. Enabled by slot-based locals making such functions `slot_based`.
//!
//! Anything unsupported (member/index, calls, arrays/objects, non-number constants, pow,
//! booleans in slots, a ternary inside a loop) makes compilation return `None`, and any non-number
//! argument at call time falls back to the interpreter — so this is purely ADDITIVE and can never
//! change behaviour (a miss runs the VM). Only a logic bug here could, hence the differential
//! validation (vm-JIT ≡ interp ≡ node) + `tests/core/jit_loops.tish`, `tests/core/jit_shifts.tish`.
//! Bitwise `& | ^ ~` and shifts `<< >> >>>` are lowered (JS ToInt32/ToUint32 semantics, bit-exact
//! with the VM's `eval_binop`); only `**` (pow) still falls back.
//!
//! Not compiled for wasm targets (cranelift-jit emits host code).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use cranelift::codegen::settings::{self, Configurable};
use cranelift::prelude::types;
use cranelift::prelude::{
    AbiParam, Block, FloatCC, FunctionBuilder, FunctionBuilderContext, InstBuilder, IntCC,
    MemFlags, Value as ClifValue, Variable,
};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use tishlang_ast::{BinOp, UnaryOp};
use tishlang_bytecode::{
    u8_to_binop, u8_to_unaryop, Chunk, Constant, MathUnaryFn, Opcode, NO_REST_PARAM,
};

/// A JIT-compiled hot LOOP REGION for on-stack replacement (#190). Unlike [`NumericFn`] (a whole
/// function over register-`f64` params), this is native code over a chunk's **numeric slot frame**:
/// the VM copies the region's live-in slots into an `f64` buffer, calls the region, then copies the
/// live-outs back. ABI: `extern "C" fn(slots: *mut f64, deopt: *mut u8) -> i32`, returning the EXIT
/// id (index into [`exits`]). `deopt` is reserved (a `*mut u8` flag): the v1 whitelist is pure numeric
/// slot math, which cannot deopt mid-run, so it is never set — it keeps the ABI stable for the future
/// array/property regions that will need a bail path. The pointer is into the never-freed `JITModule`.
#[derive(Clone)]
pub struct LoopFn {
    ptr: usize,
    /// Slots read or written inside the region — the live set. The `f64` buffer is indexed by
    /// POSITION here (buffer slot `p` ↔ chunk slot `used_slots[p]`); the emitted code loads/stores at
    /// that same position, so the VM and the native code agree without threading slot numbers.
    pub used_slots: Vec<u16>,
    /// Exit id → the chunk `ip` to resume interpreting at (a loop-exit / `break` target outside the
    /// region). The region always has ≥1 exit (an exit-less region would be an uninterruptible native
    /// loop, so compilation bails).
    pub exits: Vec<usize>,
}

// SAFETY: identical to `NumericFn` — `ptr` is immutable executable code in the process-global,
// never-dropped `JITModule`; the slot buffer is caller-owned. Send/Sync so the frame VM (which may
// run on any thread) can hold a cached `LoopFn`.
unsafe impl Send for LoopFn {}
unsafe impl Sync for LoopFn {}

impl LoopFn {
    /// Run the region. `buf` holds the live-ins (`used_slots.len()` `f64`s), updated in place with the
    /// live-outs on return. `deopt` is a 1-byte flag (unused in v1). Returns the exit id. Safe wrapper
    /// — the raw-pointer transmute (same soundness as [`NumericFn::call`]: immutable native code with a
    /// fixed C ABI) is confined here, so call sites need no `unsafe`.
    #[inline]
    pub fn call(&self, buf: &mut [f64], deopt: &mut u8) -> i32 {
        // SAFETY: `ptr` is immutable executable code compiled for exactly this `(*mut f64, *mut u8)`
        // ABI; `buf`/`deopt` are valid for the call and the region only touches `buf[0..used_slots]`.
        unsafe {
            let f: extern "C" fn(*mut f64, *mut u8) -> i32 = std::mem::transmute(self.ptr);
            f(buf.as_mut_ptr(), deopt as *mut u8)
        }
    }
}

/// A JIT-compiled numeric function: a pointer to native code plus its arity.
/// The pointer is into a leaked, never-freed `JITModule`, so it is valid for the
/// life of the process and safe to call from any thread.
#[derive(Clone, Copy)]
pub struct NumericFn {
    ptr: usize,
    arity: u8,
    /// True when the function's result is a comparison (boolean), so the caller
    /// boxes the returned f64 as `Value::Bool` (1.0→true) instead of
    /// `Value::Number`. Needed for callbacks like `x => x === c` used by `map`.
    result_bool: bool,
    /// Array-param bitmask (bit k set ⇒ param k is an ARRAY, read as `arr[i]`). `0` ⇒ the ordinary
    /// pure-numeric register-`f64` ABI (the [`NumericFn::call`] path, unchanged). Nonzero ⇒ the
    /// array-mode uniform 3-pointer ABI (the [`NumericFn::call_arrays`] path). Kept as a `u8` (arity
    /// ≤ 8) so `NumericFn` stays `Copy`. `TISH_JIT_ARRAYS`-gated; `0` in every default build.
    array_param_mask: u8,
    /// #187: subset of `array_param_mask` whose arrays are WRITTEN (`arr[i] = v`). After a non-deopt
    /// `call_arrays`, [`try_call_array_jit`] copies each such scratch buffer back into the caller's
    /// `Value::Array`. `0` ⇒ every array param is read-only (no writeback).
    array_writable_mask: u8,
    /// #187: subset of `array_writable_mask` whose elements are written with a BOOL const (`arr[i]=true`)
    /// rather than a number. The JIT flattens elements to `f64`, so the writeback re-boxes by the array's
    /// ORIGINAL element type — sound only when the WRITTEN kind matches that entry type. `try_call_array_jit`
    /// bails to the interpreter when a bool-written array is passed a number array (or vice versa); a mix of
    /// bool + non-bool writes to one array is rejected at compile time (`classify_params` returns `(0,0,0)`).
    array_bool_write_mask: u8,
    /// True when this is a self-recursive function compiled with a trailing `*mut RecurGuard` param
    /// (the recursion-depth bail, #381). Such a function must be invoked via [`NumericFn::call_guarded`];
    /// non-recursive functions keep the plain ABI and [`NumericFn::call`] (zero overhead).
    recur_guarded: bool,
    /// True when this function has JIT'd local `f64` arrays (#189). It uses the plain register-`f64`
    /// [`NumericFn::call`] ABI; an out-of-bounds array access (or a non-numeric return) sets a
    /// per-thread deopt flag ([`jv_take_deopt`]) and the caller discards the result + re-interprets.
    jv: bool,
    /// #187: true when this function embeds a native call to a registered callee. Such a function is
    /// NOT cached by [`try_compile_numeric`] (its embedded callee address could go stale if a
    /// long-lived process reuses chunk addresses across programs) — it is recompiled per closure
    /// creation, which resolves against the live callee registry. `false` (cacheable) for all others.
    uses_xcall: bool,
    /// #187: true when this is a VOID array-mode function (only returns the implicit `null`). Its
    /// `f64` result is a dummy, so [`try_call_array_jit`] returns `Value::Null` instead of a number.
    void_result: bool,
}

/// A flat numeric array handed to an array-mode JIT function: a raw `f64` slice (`ptr`, `len`).
/// Built by the VM wrapper by extracting an all-numeric `Array` into a scratch `Vec<f64>` (the
/// wrapper only builds one when every element is a `Value::Number`, so the slice is always valid
/// `f64`). `ptr` is `*mut` because #187 array-param WRITES store through it into the scratch buffer;
/// read-only params only load, so the mut provenance is harmless there.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArrayHandle {
    pub ptr: *mut f64,
    pub len: usize,
}

/// Recursion guard handed to a self-recursive JIT'd numeric function (#381). tish's JIT is an
/// additive, bail-to-interpreter tier, so rather than throw from inside JIT'd code (V8/JSC's model),
/// a too-deep recursion simply BAILS — exactly the deopt path array-mode already uses for an
/// out-of-bounds read: the function compares its stack pointer to `stack_limit` at entry, and if it
/// has crossed it (recursion approaching stack exhaustion) it stores `tripped = 1` and returns a
/// sentinel instead of recursing further. `VmClosure::call` then raises the catchable `RangeError`
/// through the normal pending-throw path. A single stack-pointer compare per call, sized from the
/// REAL remaining stack (`stacker::remaining_stack`) — never a per-call counter, so the hot recursion
/// (fib/spectral_norm) is untaxed. `#[repr(C)]`: `stack_limit` at offset 0, `tripped` at offset 8.
#[repr(C)]
pub struct RecurGuard {
    pub stack_limit: usize,
    pub tripped: u8,
}

/// Array-element reads inside JIT'd loops (`arr[i]`/`arr[const]`). **Default ON**; `TISH_JIT_ARRAYS=0`
/// disables it (escape hatch). Cached in a `OnceLock` — NEVER read the env var on a hot path (see the
/// frame-VM regression note in docs/perf.md). Purely ADDITIVE: only numeric-array-reduction functions
/// are array-compiled, and the VM wrapper bails to the interpreter for any non-numeric element,
/// `NumberArray`, or out-of-bounds deopt — so a non-fast case is always correct, just interpreted.
/// Validated: full cross-backend suite 17/0 both ON and OFF; vm(JIT) ≡ interp ≡ node on
/// sum/dot/max/const-index/OOB/non-numeric/float fixtures; 38× on `sumArr`-style reductions.
#[cfg(not(target_arch = "wasm32"))]
pub fn jit_arrays_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("TISH_JIT_ARRAYS")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// JIT-compiled function-LOCAL `f64` arrays via the `tish_jv_*` runtime (#189). **Default ON**;
/// `TISH_JIT_JV=0` disables it (escape hatch). Additive: a function whose arrays don't fit the
/// non-escaping-JV shape just runs interpreted, and any out-of-bounds access deopts to the interpreter.
#[cfg(not(target_arch = "wasm32"))]
pub fn jit_jv_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("TISH_JIT_JV")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// Boolean scalar local slots in the numeric CFG JIT (#187). **Default ON**; `TISH_JIT_BOOL_SLOTS=0`
/// disables it. A `let flag = false` / `flag = true` / `if (flag)` local is represented as an `f64`
/// `0.0`/`1.0`; a syntactic pre-pass ([`classify_bool_slots`]) tags the slots, and the equality
/// guard in [`emit_simple_op`] + the return-of-bool bail keep it sound (a bool never reaches a
/// diverging `bool === number` compare or a boolean function result). Unblocks e.g. fannkuch.
pub fn jit_bool_slots_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("TISH_JIT_BOOL_SLOTS")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// The self-recursion stack guard (#381). **Default ON**; `TISH_JIT_RECUR_GUARD=0` disables it.
///
/// A JIT'd self-recursive numeric function recurses on the native stack (SelfCall lowers to a native
/// call), bypassing the VM's `inc_call_depth` ceiling. Without a guard, unbounded numeric recursion
/// overflows the native stack — an uncatchable `SIGSEGV`/abort that takes down the whole worker
/// process (all in-flight requests on it), the DoS hole #381 exists to close. The guard adds a
/// trailing `*mut RecurGuard` param whose entry compares the stack pointer to a per-thread limit and
/// bails (→ a catchable `RangeError`, like the interp/VM paths) before overflow. This is like Go's
/// per-prologue stack check — cheap safety for a server language — but the extra param must stay live
/// across the recursive calls, so it costs ~12-34% on hot numeric recursion (worst on trivial bodies
/// like `fib`). Trusted, provably-bounded hot recursion can opt out for raw speed. Cached in a
/// `OnceLock`; the env read is off the hot path (compile time only), never per call.
#[cfg(not(target_arch = "wasm32"))]
pub fn jit_recur_guard_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("TISH_JIT_RECUR_GUARD")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

// SAFETY: `ptr` references immutable executable code in a module that is never
// dropped (it lives in the process-global `JIT` below). All mutation of the
// module happens under the `Mutex`; only the raw code pointer escapes.
unsafe impl Send for NumericFn {}
unsafe impl Sync for NumericFn {}

impl NumericFn {
    #[inline]
    pub fn arity(&self) -> usize {
        self.arity as usize
    }

    /// Whether the result should be boxed as `Value::Bool` rather than `Number`.
    #[inline]
    pub fn result_is_bool(&self) -> bool {
        self.result_bool
    }

    /// Call the native function. `args.len()` must equal `arity`.
    #[inline]
    pub fn call(&self, args: &[f64]) -> f64 {
        // The module's default call conv matches the C ABI for `f64` scalars on
        // x86-64 SysV and AArch64, so transmuting to `extern "C" fn` is sound here.
        unsafe {
            match self.arity {
                1 => {
                    let f: extern "C" fn(f64) -> f64 = std::mem::transmute(self.ptr);
                    f(args[0])
                }
                2 => {
                    let f: extern "C" fn(f64, f64) -> f64 = std::mem::transmute(self.ptr);
                    f(args[0], args[1])
                }
                3 => {
                    let f: extern "C" fn(f64, f64, f64) -> f64 = std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2])
                }
                // Arities 4..=8: still fully register-passed (x86-64 SysV → XMM0-7,
                // AArch64 AAPCS → V0-7), so the `extern "C"` transmute stays sound.
                4 => {
                    let f: extern "C" fn(f64, f64, f64, f64) -> f64 = std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3])
                }
                5 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], args[4])
                }
                6 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64, f64) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], args[4], args[5])
                }
                7 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64, f64, f64) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6],
                    )
                }
                8 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64, f64, f64, f64) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    )
                }
                _ => f64::NAN,
            }
        }
    }

    /// Whether this function was compiled with a `RecurGuard` param (self-recursive). If so it MUST be
    /// invoked via [`call_guarded`], not [`call`] (the ABI has the trailing pointer param). #381
    #[inline]
    pub fn recur_guarded(&self) -> bool {
        self.recur_guarded
    }

    /// Call a self-recursive (`recur_guarded`) function, passing the `RecurGuard` as a trailing pointer
    /// param. On return the caller inspects `guard.tripped`: if set, the recursion hit the stack limit
    /// and bailed (the numeric result is a discarded sentinel) → raise a catchable `RangeError`. #381
    #[inline]
    pub fn call_guarded(&self, args: &[f64], guard: *mut RecurGuard) -> f64 {
        // Same `extern "C"` f64-register ABI as `call`, plus a trailing pointer arg for the guard.
        unsafe {
            match self.arity {
                1 => {
                    let f: extern "C" fn(f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], guard)
                }
                2 => {
                    let f: extern "C" fn(f64, f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], guard)
                }
                3 => {
                    let f: extern "C" fn(f64, f64, f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], guard)
                }
                4 => {
                    let f: extern "C" fn(f64, f64, f64, f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], guard)
                }
                5 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], args[4], guard)
                }
                6 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64, f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], args[4], args[5], guard)
                }
                7 => {
                    let f: extern "C" fn(
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        *mut RecurGuard,
                    ) -> f64 = std::mem::transmute(self.ptr);
                    f(
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], guard,
                    )
                }
                8 => {
                    let f: extern "C" fn(
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        f64,
                        *mut RecurGuard,
                    ) -> f64 = std::mem::transmute(self.ptr);
                    f(
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                        guard,
                    )
                }
                _ => f64::NAN,
            }
        }
    }

    /// Whether this function has JIT'd local arrays (#189). Such a function uses the ordinary
    /// register-`f64` [`call`] ABI — its OOB/deopt signal is the per-thread flag (see
    /// [`jv_reset_deopt`] / [`jv_take_deopt`]), not a trailing pointer param.
    #[inline]
    pub fn is_jv(&self) -> bool {
        self.jv
    }

    /// Bit k set ⇒ param k is an array param (read via `arr[i]`). `0` ⇒ pure-numeric (use [`call`]).
    #[inline]
    pub fn array_param_mask(&self) -> u8 {
        self.array_param_mask
    }

    /// #187: bit k set ⇒ array param k is WRITTEN (`arr[i] = v`) and its scratch buffer must be copied
    /// back into the caller's `Value::Array` after a non-deopt run. Subset of [`array_param_mask`].
    #[inline]
    pub fn array_writable_mask(&self) -> u8 {
        self.array_writable_mask
    }

    /// #187: bit k set ⇒ writable array param k is written with BOOL consts (`arr[i]=true`), not numbers.
    /// The writeback re-boxes by the entry array's element type, so [`try_call_array_jit`] must bail when
    /// a bool-written array receives a number array (or a number-written array receives a bool array).
    #[inline]
    pub fn array_bool_write_mask(&self) -> u8 {
        self.array_bool_write_mask
    }

    /// #187: true when this array-mode function is VOID (returns the implicit `null`); its `f64` result
    /// is a dummy, so [`try_call_array_jit`] returns `Value::Null` for it.
    #[inline]
    pub fn returns_void(&self) -> bool {
        self.void_result
    }

    /// Call an array-mode function (`array_param_mask != 0`). `numeric` holds the f64 values for the
    /// numeric params in numeric-param order; `arrays` the [`ArrayHandle`]s for the array params in
    /// array-param order. Returns `(result, deopt)` — when `deopt` is true an out-of-bounds access
    /// was hit and the JIT bailed, so the caller MUST discard `result` and re-run the interpreter
    /// (OOB reads return `Value::Null` in the VM, whose per-operator coercion the JIT can't replicate).
    #[inline]
    pub fn call_arrays(&self, numeric: &[f64], arrays: &[ArrayHandle]) -> (f64, bool) {
        self.call_arrays_guarded(numeric, arrays, std::ptr::null_mut())
    }

    /// #187: array-mode call, optionally passing a trailing `*mut RecurGuard` (non-null iff
    /// `recur_guarded()` — a self-recursive array fn). On return the caller inspects `guard.tripped`:
    /// set ⇒ the native recursion hit the stack limit and bailed (result is a sentinel) → RangeError.
    #[inline]
    pub fn call_arrays_guarded(
        &self,
        numeric: &[f64],
        arrays: &[ArrayHandle],
        guard: *mut RecurGuard,
    ) -> (f64, bool) {
        let mut deopt: u8 = 0;
        // ONE uniform signature for every array-mode fn: (numeric*, handles*, deopt*[, guard*]) -> f64.
        // Empty slices pass a dangling-but-aligned non-null ptr (the body only loads indices it uses).
        let num_ptr = if numeric.is_empty() {
            std::ptr::NonNull::<f64>::dangling().as_ptr() as *const f64
        } else {
            numeric.as_ptr()
        };
        let arr_ptr = if arrays.is_empty() {
            std::ptr::NonNull::<ArrayHandle>::dangling().as_ptr() as *const ArrayHandle
        } else {
            arrays.as_ptr()
        };
        let res = unsafe {
            if guard.is_null() {
                let f: extern "C" fn(*const f64, *const ArrayHandle, *mut u8) -> f64 =
                    std::mem::transmute(self.ptr);
                f(num_ptr, arr_ptr, &mut deopt as *mut u8)
            } else {
                let f: extern "C" fn(
                    *const f64,
                    *const ArrayHandle,
                    *mut u8,
                    *mut RecurGuard,
                ) -> f64 = std::mem::transmute(self.ptr);
                f(num_ptr, arr_ptr, &mut deopt as *mut u8, guard)
            }
        };
        (res, deopt != 0)
    }
}

struct JitGlobal {
    module: JITModule,
    /// Keyed by the address of the nested `Chunk`, with a content **fingerprint** alongside the
    /// result. Within one program run a chunk lives for the whole run, so the address is stable and
    /// unique. But this cache is a process-global that is never cleared, and a `Chunk` is dropped when
    /// its program is — so a long-lived process that compiles/drops/recompiles programs (the REPL;
    /// embedders running multiple scripts) can allocate a *different* chunk at a freed address that is
    /// still cached. We therefore verify the fingerprint on every hit: a mismatch means the address was
    /// reused by a different chunk, so we recompile (and overwrite) instead of returning stale native
    /// code. `None` still caches "not JIT-eligible". See [`chunk_fingerprint`].
    cache: HashMap<usize, (u64, Option<NumericFn>)>,
    /// OSR loop-region cache (#190), keyed by `(chunk address, loop header ip)` with the same
    /// fingerprint guard as `cache`. `None` caches "region not compilable" so a loop that fails the
    /// whitelist is scanned once, not on every back-edge past the trigger threshold.
    osr_cache: HashMap<(usize, usize), (u64, Option<LoopFn>)>,
    counter: usize,
    /// `FuncId` of the imported `tish_math_call` host fn (#186), declared once at module init and
    /// re-imported into each compiled function via `declare_func_in_func`.
    math_call_id: cranelift_module::FuncId,
    /// `FuncId`s of the imported `tish_jv_*` vector runtime (#189).
    jv_fns: JvFns,
    /// #187: directly-callable numeric callees, keyed by the stable global name a top-level function is
    /// bound to (`Chunk.global_name`). Populated when a plain register-`f64` function compiles; a
    /// caller's `name(args)` then lowers to a native cranelift call to `id`. Sound because the compiler
    /// only stamps `global_name` on functions it proved are never reassigned/shadowed program-wide, so
    /// the binding can never change under a cached caller. A caller that references a name NOT yet here
    /// (a forward reference) simply bails to the interpreter.
    callees: HashMap<Arc<str>, CalleeEntry>,
}

/// #187: a registered directly-callable numeric callee (register-`f64` ABI). Callers resolve against
/// the LIVE registry at compile time and are never cached ([`NumericFn::uses_xcall`]), so a name
/// re-registered by a later program simply overwrites this — a stale callee is never invoked.
#[derive(Clone, Copy)]
struct CalleeEntry {
    id: cranelift_module::FuncId,
    arity: u8,
}

// SAFETY: `JITModule` is `!Send`, but the single instance lives behind the
// `Mutex` in the process-global `JIT` and is never moved out or dropped; all
// access is serialized by the mutex.
unsafe impl Send for JitGlobal {}

static JIT: OnceLock<Option<Mutex<JitGlobal>>> = OnceLock::new();

// #189 — a minimal C-ABI `f64` vector runtime for JIT-compiled function-LOCAL arrays that never
// escape the frame. A qualifying local slot (only def = empty `[]`, only uses = push / `[i]` /
// `[i]=v` / `.length`) is addressed by a `u64` HANDLE into a per-thread arena instead of a boxed
// `Value::Array`, so its index math is a bounds-checked slab lookup, not a per-op bound-method
// alloc — and, because the arena is a plain `Vec<Vec<f64>>` behind a `RefCell`, the host fns need
// NO `unsafe` (no raw-pointer deref). SOUND because such arrays never escape: no `Value` ever
// references a `Vec`, so an out-of-bounds access can set the deopt flag, let the (about-to-be-
// discarded) native run finish, and re-run the interpreter with no observable state change. Handle
// `0` is the null sentinel (a JV slot inits to 0); every exit frees every live slot back to the
// free list, so the arena is bounded by a function's peak live-array count, not total allocations.
#[cfg(not(target_arch = "wasm32"))]
struct JvArena {
    vecs: Vec<Vec<f64>>,
    free: Vec<usize>,
    deopt: bool,
}
#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static JV_ARENA: std::cell::RefCell<JvArena> = const {
        std::cell::RefCell::new(JvArena { vecs: Vec::new(), free: Vec::new(), deopt: false })
    };
}
/// `handle` (1-based, `0` = null) → arena index, or `None` for the null handle.
#[cfg(not(target_arch = "wasm32"))]
#[inline]
fn jv_index(handle: u64) -> Option<usize> {
    handle.checked_sub(1).map(|i| i as usize)
}
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_new(cap: u64) -> u64 {
    JV_ARENA.with(|c| {
        let mut a = c.borrow_mut();
        let v = Vec::with_capacity(cap as usize);
        let idx = match a.free.pop() {
            Some(i) => {
                a.vecs[i] = v;
                i
            }
            None => {
                a.vecs.push(v);
                a.vecs.len() - 1
            }
        };
        idx as u64 + 1 // handle = index + 1 (0 stays reserved for null)
    })
}
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_push(handle: u64, x: f64) {
    if let Some(idx) = jv_index(handle) {
        JV_ARENA.with(|c| {
            if let Some(v) = c.borrow_mut().vecs.get_mut(idx) {
                v.push(x);
            }
        });
    }
}
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_get(handle: u64, i: u64) -> f64 {
    JV_ARENA.with(|c| {
        let mut a = c.borrow_mut();
        match jv_index(handle)
            .and_then(|idx| a.vecs.get(idx))
            .and_then(|v| v.get(i as usize))
        {
            Some(&x) => x,
            None => {
                a.deopt = true; // OOB (or null): caller discards the result + re-interprets
                f64::NAN
            }
        }
    })
}
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_set(handle: u64, i: u64, x: f64) {
    JV_ARENA.with(|c| {
        let mut a = c.borrow_mut();
        match jv_index(handle)
            .and_then(|idx| a.vecs.get_mut(idx))
            .and_then(|v| v.get_mut(i as usize))
        {
            Some(p) => *p = x,
            None => a.deopt = true, // OOB (or null) → deopt (no store performed)
        }
    })
}
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_len(handle: u64) -> u64 {
    JV_ARENA.with(|c| {
        jv_index(handle)
            .and_then(|idx| c.borrow().vecs.get(idx).map(|v| v.len() as u64))
            .unwrap_or(0)
    })
}
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_free(handle: u64) {
    if let Some(idx) = jv_index(handle) {
        JV_ARENA.with(|c| {
            let mut a = c.borrow_mut();
            if let Some(v) = a.vecs.get_mut(idx) {
                v.clear(); // keep capacity; the slot is returned to the free list for reuse
                a.free.push(idx);
            }
        });
    }
}
/// Signal a deopt from JIT'd code that can't produce an `f64` result (a non-numeric `return`, e.g. the
/// dead `return null` epilogue after a `while (true)`); the wrapper then re-interprets. #189
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_jv_deopt() {
    JV_ARENA.with(|c| c.borrow_mut().deopt = true);
}
/// Clear the per-thread deopt flag before entering a JV function. #189
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn jv_reset_deopt() {
    JV_ARENA.with(|c| c.borrow_mut().deopt = false);
}
/// Read and clear the deopt flag after a JV function returns; `true` ⇒ discard its result + interpret.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn jv_take_deopt() -> bool {
    JV_ARENA.with(|c| {
        let mut a = c.borrow_mut();
        std::mem::replace(&mut a.deopt, false)
    })
}

/// Imported `FuncId`s of the `tish_jv_*` vector runtime (#189), declared once at module init.
#[derive(Clone, Copy)]
struct JvFns {
    new: cranelift_module::FuncId,
    push: cranelift_module::FuncId,
    get: cranelift_module::FuncId,
    set: cranelift_module::FuncId,
    len: cranelift_module::FuncId,
    free: cranelift_module::FuncId,
    deopt: cranelift_module::FuncId,
}

/// Per-function-build handles for JV local arrays (#189): the `tish_jv_*` `FuncRef`s imported into
/// *this* function, plus the set of slots classified as JV arrays. Passed to [`build_body_cfg`].
/// The deopt flag lives in the per-thread arena (set by `get`/`set`/`deopt`), so — unlike array
/// mode — the JV function ABI carries no trailing deopt pointer.
struct JvCtx<'a> {
    new: cranelift::codegen::ir::FuncRef,
    push: cranelift::codegen::ir::FuncRef,
    get: cranelift::codegen::ir::FuncRef,
    set: cranelift::codegen::ir::FuncRef,
    len: cranelift::codegen::ir::FuncRef,
    free: cranelift::codegen::ir::FuncRef,
    deopt: cranelift::codegen::ir::FuncRef,
    slots: &'a std::collections::HashSet<usize>,
}

/// Host entry point the JIT calls for `Math.<fn>` intrinsics it doesn't lower to a native op (#186):
/// sin/cos/tan/exp/log/round/sign/… The single source of truth is [`MathUnaryFn::apply`], so JIT ≡
/// VM ≡ interpreter bit-for-bit. `extern "C"` and registered as a JIT symbol; the id is the operand.
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn tish_math_call(id: i32, x: f64) -> f64 {
    match tishlang_bytecode::MathUnaryFn::from_u16(id as u16) {
        Some(m) => m.apply(x),
        None => f64::NAN,
    }
}

/// Build the JIT module and declare the imported host functions (`tish_math_call` #186, the
/// `tish_jv_*` vector runtime #189), returning the module + their `FuncId`s.
fn new_module() -> Option<(JITModule, cranelift_module::FuncId, JvFns)> {
    let mut flag_builder = settings::builder();
    // JIT code is loaded at a fixed address; no PIC / colocated libcalls needed.
    flag_builder.set("use_colocated_libcalls", "false").ok()?;
    flag_builder.set("is_pic", "false").ok()?;
    flag_builder.set("opt_level", "speed").ok()?;
    let isa_builder = cranelift_native::builder().ok()?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .ok()?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    builder.symbol("tish_math_call", tish_math_call as *const u8);
    builder.symbol("tish_jv_new", tish_jv_new as *const u8);
    builder.symbol("tish_jv_push", tish_jv_push as *const u8);
    builder.symbol("tish_jv_get", tish_jv_get as *const u8);
    builder.symbol("tish_jv_set", tish_jv_set as *const u8);
    builder.symbol("tish_jv_len", tish_jv_len as *const u8);
    builder.symbol("tish_jv_free", tish_jv_free as *const u8);
    builder.symbol("tish_jv_deopt", tish_jv_deopt as *const u8);
    let mut module = JITModule::new(builder);

    // `tish_math_call(i32 fn-id, f64 x) -> f64`.
    let mut msig = module.make_signature();
    msig.params.push(AbiParam::new(types::I32));
    msig.params.push(AbiParam::new(types::F64));
    msig.returns.push(AbiParam::new(types::F64));
    let math_id = module
        .declare_function("tish_math_call", Linkage::Import, &msig)
        .ok()?;

    // Helper to declare a `tish_jv_*` import from param/return abi lists.
    let mut declare = |name: &str, params: &[AbiParam], rets: &[AbiParam]| {
        let mut s = module.make_signature();
        s.params.extend_from_slice(params);
        s.returns.extend_from_slice(rets);
        module.declare_function(name, Linkage::Import, &s).ok()
    };
    // A JV array is addressed by an `i64` HANDLE (a per-thread arena index; `0` = null), not a raw
    // pointer. `get`/`set` need no deopt-pointer arg — OOB sets a per-thread flag the wrapper reads.
    let jv = JvFns {
        new: declare(
            "tish_jv_new",
            &[AbiParam::new(types::I64)],
            &[AbiParam::new(types::I64)],
        )?,
        push: declare(
            "tish_jv_push",
            &[AbiParam::new(types::I64), AbiParam::new(types::F64)],
            &[],
        )?,
        get: declare(
            "tish_jv_get",
            &[AbiParam::new(types::I64), AbiParam::new(types::I64)],
            &[AbiParam::new(types::F64)],
        )?,
        set: declare(
            "tish_jv_set",
            &[
                AbiParam::new(types::I64),
                AbiParam::new(types::I64),
                AbiParam::new(types::F64),
            ],
            &[],
        )?,
        len: declare(
            "tish_jv_len",
            &[AbiParam::new(types::I64)],
            &[AbiParam::new(types::I64)],
        )?,
        free: declare("tish_jv_free", &[AbiParam::new(types::I64)], &[])?,
        deopt: declare("tish_jv_deopt", &[], &[])?,
    };
    Some((module, math_id, jv))
}

fn jit() -> Option<&'static Mutex<JitGlobal>> {
    JIT.get_or_init(|| {
        new_module().map(|(module, math_call_id, jv_fns)| {
            Mutex::new(JitGlobal {
                module,
                cache: HashMap::new(),
                osr_cache: HashMap::new(),
                counter: 0,
                math_call_id,
                jv_fns,
                callees: HashMap::new(),
            })
        })
    })
    .as_ref()
}

/// Read a big-endian u16 operand (matches the VM/compiler encoding).
#[inline]
fn read_u16(code: &[u8], ip: &mut usize) -> Option<u16> {
    let a = *code.get(*ip)? as u16;
    let b = *code.get(*ip + 1)? as u16;
    *ip += 2;
    Some((a << 8) | b)
}

/// Content fingerprint of everything `compile_chunk` reads, so a cache entry can be validated against
/// the chunk currently at a (possibly reused) address. FNV-1a over the compile-relevant fields:
/// shape (`param_count`, `num_slots`, `rest_param_index`, `slot_based`), the full `code` bytes, and
/// the `constants` (the JIT emits `f64const`/bool from `LoadConst`, so their values matter). The JIT
/// makes no cross-chunk calls (`op_size` allows only `SelfCall`, which recurses into *this* function),
/// so nothing outside the chunk affects the result — this fingerprint is complete. Deterministic
/// within a process (fixed FNV constants, not a randomized hasher), which is all the cache needs.
fn chunk_fingerprint(chunk: &Chunk) -> u64 {
    // Mixes a u64 at a time (FNV-prime multiply + an avalanche shift). Eight bytes per round keeps
    // this cheap on the hot closure-creation path; correctness only needs determinism + good
    // distinction, not cryptographic strength.
    #[inline]
    fn mix(h: &mut u64, v: u64) {
        *h ^= v;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        *h ^= *h >> 29;
    }
    #[inline]
    fn mix_bytes(h: &mut u64, bytes: &[u8]) {
        let mut it = bytes.chunks_exact(8);
        for w in &mut it {
            mix(h, u64::from_le_bytes(w.try_into().unwrap()));
        }
        let rem = it.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            mix(h, u64::from_le_bytes(buf));
        }
        mix(h, bytes.len() as u64);
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    mix(&mut h, chunk.param_count as u64);
    mix(&mut h, chunk.num_slots as u64);
    mix(&mut h, chunk.rest_param_index as u64);
    mix(&mut h, chunk.slot_based as u64);
    mix_bytes(&mut h, &chunk.code);
    mix(&mut h, chunk.constants.len() as u64);
    for c in &chunk.constants {
        match c {
            Constant::Number(n) => {
                mix(&mut h, 1);
                mix(&mut h, n.to_bits());
            }
            Constant::String(s) => {
                mix(&mut h, 2);
                mix_bytes(&mut h, s.as_bytes());
            }
            Constant::Bool(b) => mix(&mut h, if *b { 4 } else { 3 }),
            Constant::Null => mix(&mut h, 5),
            Constant::Closure(idx) => {
                mix(&mut h, 6);
                mix(&mut h, *idx as u64);
            }
        }
    }
    h
}

/// Get (or compile, then cache) the native numeric function for `chunk`.
/// Returns `None` if the chunk isn't a straight-line numeric function.
/// #187: clear the directly-callable-callee registry at the start of each top-level program run, so a
/// long-lived process (REPL / embedder) never resolves a callee registered by a PRIOR program (a name
/// re-registered non-numerically would otherwise leave a stale native entry). Cross-callers aren't
/// cached, so they always re-resolve against the freshly-populated registry.
#[cfg(not(target_arch = "wasm32"))]
pub fn reset_callees() {
    if let Some(lock) = jit() {
        if let Ok(mut g) = lock.lock() {
            g.callees.clear();
        }
    }
}

pub fn try_compile_numeric(chunk: &Chunk) -> Option<NumericFn> {
    if !chunk.slot_based
        || chunk.rest_param_index != NO_REST_PARAM
        || chunk.param_count == 0
        || chunk.param_count > 8
    {
        return None;
    }
    let key = chunk as *const Chunk as usize;
    let fp = chunk_fingerprint(chunk);
    let lock = jit()?;
    let mut g = lock.lock().ok()?;
    // Hit only counts if the fingerprint matches: otherwise this address was freed and reused by a
    // *different* chunk, and the cached `NumericFn` is native code for the old one (a miscompile).
    if let Some(&(cached_fp, cached)) = g.cache.get(&key) {
        if cached_fp == fp {
            return cached;
        }
    }
    let result = compile_chunk(&mut g, chunk);
    // #187: a function that embeds a native call to a registered callee is NOT cached — its callee
    // address could go stale across programs in a long-lived process. It recompiles per closure
    // creation (once, in practice), resolving against the live registry. Everything else caches.
    if !result.is_some_and(|nf| nf.uses_xcall) {
        g.cache.insert(key, (fp, result));
    }
    result
}

/// Compile the hot loop region `[header_ip, region_end)` of `chunk` to native code (#190 OSR), or
/// `None` if it is not a pure-numeric slot loop. Cached per `(chunk, header_ip)` with a fingerprint
/// guard (negative results included, so a non-compilable loop is scanned once). Called from the frame
/// VM's `JumpBack` handler once a loop's back-edge counter crosses the trigger threshold.
pub fn try_compile_loop(chunk: &Chunk, header_ip: usize, region_end: usize) -> Option<LoopFn> {
    let key = (chunk as *const Chunk as usize, header_ip);
    let fp = chunk_fingerprint(chunk);
    let lock = jit()?;
    let mut g = lock.lock().ok()?;
    if let Some((cached_fp, cached)) = g.osr_cache.get(&key) {
        if *cached_fp == fp {
            return cached.clone();
        }
    }
    let result = compile_loop_region(&mut g, chunk, header_ip, region_end);
    g.osr_cache.insert(key, (fp, result.clone()));
    result
}

/// Lower one hot loop region to a `LoopFn`. The region must be pure numeric slot math: the same
/// opcode whitelist as [`build_body_cfg`] minus everything that touches a non-slot value (calls,
/// member/index, arrays, objects, `LoadVar`, `Return`). Reuses [`emit_simple_op`] for the arithmetic
/// so the region is bit-for-bit identical to the interpreter's `eval_binop`. Live-ins are loaded from
/// the `slots` pointer at entry; each loop-exit target stores the live set back and returns its id.
fn compile_loop_region(
    g: &mut JitGlobal,
    chunk: &Chunk,
    header_ip: usize,
    region_end: usize,
) -> Option<LoopFn> {
    let code = &chunk.code;
    let num_slots = chunk.num_slots as usize;
    if num_slots == 0 || num_slots > 256 || header_ip >= region_end || region_end > code.len() {
        return None;
    }

    // 1. Scan the region: validate the whitelist (op_size = None ⇒ bail), collect in-region block
    //    leaders, the live slot set, and the EXIT targets (jump targets outside the region). A
    //    JumpBack must stay inside the region (its own loop, possibly nested); one leaving the region
    //    is not a structured single-region loop → bail.
    let mut leaders: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    leaders.insert(header_ip);
    let mut used: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    let mut exit_targets: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut ip = header_ip;
    while ip < region_end {
        let op = Opcode::from_u8(*code.get(ip)?)?;
        match op {
            // Pure slot / stack / arithmetic / structured control flow — the region vocabulary.
            Opcode::Nop
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::EnterBlock
            | Opcode::ExitBlock
            | Opcode::LoopVarsEnd
            | Opcode::LoopVarsBegin
            | Opcode::UnaryOp
            | Opcode::MathUnary => {} // #186 — `Math.<fn>(x)`: 1 f64 in, 1 f64 out (like UnaryOp)
            Opcode::LoadLocal | Opcode::StoreLocal => {
                used.insert(peek_u16(code, ip + 1)?);
            }
            Opcode::LoadConst => match chunk.constants.get(peek_u16(code, ip + 1)? as usize) {
                Some(Constant::Number(_)) | Some(Constant::Bool(_)) => {}
                _ => return None, // a String/Null/Closure const is not numeric slot math
            },
            Opcode::BinOp => {
                // Reject non-numeric binops up front (Pow/In/logical) so the scan and the emit agree.
                match peek_u16(code, ip + 1)
                    .map(|r| r as u8)
                    .and_then(u8_to_binop)?
                {
                    BinOp::And | BinOp::Or | BinOp::Pow | BinOp::In => return None,
                    _ => {}
                }
            }
            Opcode::Jump => {
                let off = peek_u16(code, ip + 1)? as i16 as isize;
                let t = ((ip + 3) as isize + off).max(0) as usize;
                if t < header_ip || t >= region_end {
                    exit_targets.insert(t);
                } else {
                    leaders.insert(t);
                }
            }
            Opcode::JumpIfFalse => {
                let off = peek_u16(code, ip + 1)? as i16 as isize;
                let t = ((ip + 3) as isize + off).max(0) as usize;
                if t < header_ip || t >= region_end {
                    exit_targets.insert(t);
                } else {
                    leaders.insert(t);
                }
                leaders.insert(ip + 3); // fall-through
            }
            Opcode::JumpBack => {
                let dist = peek_u16(code, ip + 1)? as usize;
                let t = (ip + 3).checked_sub(dist)?;
                if t < header_ip || t >= region_end {
                    return None; // back-edge leaving the region — not a single structured region
                }
                leaders.insert(t);
            }
            // Everything else (Call, SelfCall, LoadVar, GetIndex, member/object/array, Return, …)
            // reads or writes a non-slot Value the f64 buffer can't carry → not OSR-eligible.
            _ => return None,
        }
        ip += op_size(op)?;
    }
    // An exit-less region would compile to an uninterruptible native infinite loop — never OSR it.
    if exit_targets.is_empty() {
        return None;
    }

    // buffer position of each live slot (both the emitted loads/stores and the VM copy use this).
    let used_slots: Vec<u16> = used.iter().copied().collect();
    let buf_pos: HashMap<u16, usize> = used_slots
        .iter()
        .enumerate()
        .map(|(p, &s)| (s, p))
        .collect();
    let exits: Vec<usize> = exit_targets.iter().copied().collect();
    let exit_id: HashMap<usize, usize> = exits.iter().enumerate().map(|(i, &t)| (t, i)).collect();

    // 2. Build the region function. Signature: (slots: i64 ptr, deopt: i64 ptr) -> i32 exit id.
    let ptr_ty = g.module.target_config().pointer_type();
    let mut sig = g.module.make_signature();
    sig.params.push(AbiParam::new(ptr_ty)); // slots buffer
    sig.params.push(AbiParam::new(ptr_ty)); // deopt flag (reserved)
    sig.returns.push(AbiParam::new(types::I32));

    let name = format!("tish_osr_{}", g.counter);
    g.counter += 1;
    let id = g
        .module
        .declare_function(&name, Linkage::Export, &sig)
        .ok()?;

    let mut ctx = g.module.make_context();
    ctx.func.signature = sig.clone();
    let math_fref = g.module.declare_func_in_func(g.math_call_id, &mut ctx.func);
    let mut fbctx = FunctionBuilderContext::new();
    let built = build_loop_region_body(
        &mut ctx.func,
        &mut fbctx,
        chunk,
        header_ip,
        region_end,
        &leaders,
        &used_slots,
        &buf_pos,
        &exit_id,
        math_fref,
    );
    if !built {
        g.module.clear_context(&mut ctx);
        return None;
    }
    if g.module.define_function(id, &mut ctx).is_err() {
        g.module.clear_context(&mut ctx);
        return None;
    }
    g.module.clear_context(&mut ctx);
    if g.module.finalize_definitions().is_err() {
        return None;
    }
    let fptr = g.module.get_finalized_function(id);
    Some(LoopFn {
        ptr: fptr as usize,
        used_slots,
        exits,
    })
}

/// Emit the CLIF body of a loop region (#190). `true` on success; `false` on any shape the emitter
/// can't lower (the caller then negative-caches the region). Structure mirrors [`build_body_cfg`]'s
/// translate loop, but: slots load from / store to the `slots` pointer instead of register params,
/// and a jump/branch to an EXIT target lands in an exit block that writes the live set back and
/// returns the exit id. No self-call, no arrays, no `Return` (all rejected in the scan).
#[allow(clippy::too_many_arguments)]
fn build_loop_region_body(
    func: &mut cranelift::codegen::ir::Function,
    fbctx: &mut FunctionBuilderContext,
    chunk: &Chunk,
    header_ip: usize,
    region_end: usize,
    leaders: &std::collections::BTreeSet<usize>,
    used_slots: &[u16],
    buf_pos: &HashMap<u16, usize>,
    exit_id: &HashMap<usize, usize>,
    math_fref: cranelift::codegen::ir::FuncRef,
) -> bool {
    let code = &chunk.code;
    let num_slots = chunk.num_slots as usize;
    let mut bcx = FunctionBuilder::new(func, fbctx);

    let blocks: HashMap<usize, Block> = leaders.iter().map(|&o| (o, bcx.create_block())).collect();
    let exit_blocks: HashMap<usize, Block> =
        exit_id.keys().map(|&t| (t, bcx.create_block())).collect();

    // A jump target is either an in-region leader or a loop-exit target (the scan proved it is one).
    let target_block =
        |t: usize| -> Option<Block> { blocks.get(&t).or_else(|| exit_blocks.get(&t)).copied() };

    // Entry: load the live-in slots from the buffer, then jump into the loop header.
    let entry = bcx.create_block();
    bcx.append_block_params_for_function_params(entry);
    bcx.switch_to_block(entry);
    let params: Vec<ClifValue> = bcx.block_params(entry).to_vec();
    let slots_ptr = params[0]; // params[1] = deopt flag, reserved (v1 never writes it)
    let vars: Vec<Variable> = (0..num_slots)
        .map(|_| bcx.declare_var(types::F64))
        .collect();
    for (slot, &var) in vars.iter().enumerate() {
        // Live slots load from their buffer position; slots the region never touches init to 0 (dead).
        let init = if let Some(&p) = buf_pos.get(&(slot as u16)) {
            bcx.ins()
                .load(types::F64, MemFlags::new(), slots_ptr, (p * 8) as i32)
        } else {
            bcx.ins().f64const(0.0)
        };
        bcx.def_var(var, init);
    }
    let header_block = match blocks.get(&header_ip) {
        Some(&b) => b,
        None => return false,
    };
    bcx.ins().jump(header_block, &[]);

    // Translate the region. Operand stack is empty at every block boundary (statement-level flow).
    let mut stack: Vec<JV> = Vec::new();
    bcx.switch_to_block(header_block);
    let mut cur = header_block;
    let mut terminated = false;
    let mut ip = header_ip;
    while ip < region_end {
        if let Some(&blk) = blocks.get(&ip) {
            if blk != cur {
                if !terminated {
                    if !stack.is_empty() {
                        return false;
                    }
                    bcx.ins().jump(blk, &[]);
                }
                bcx.switch_to_block(blk);
                cur = blk;
                terminated = false;
                stack.clear();
            }
        }
        let op = match Opcode::from_u8(code[ip]).zip(op_size_at(code, ip)) {
            Some((o, _)) => o,
            None => return false,
        };
        if terminated {
            ip += match op_size(op) {
                Some(s) => s,
                None => return false,
            };
            continue;
        }
        match op {
            Opcode::LoadLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return false,
                };
                let v = match vars.get(slot) {
                    Some(v) => *v,
                    None => return false,
                };
                stack.push(JV::f64(bcx.use_var(v)));
                ip += 3;
            }
            Opcode::StoreLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return false,
                };
                let jval = match stack.pop() {
                    Some(x) => x,
                    None => return false,
                };
                if jval.is_bool() {
                    return false; // no boolean slots (keeps the number/bool distinction clean)
                }
                let v = match vars.get(slot) {
                    Some(v) => *v,
                    None => return false,
                };
                // Slots are f64 Variables — one materialize at the store boundary (int-typed
                // slots are #168's follow-up).
                let val = jv_f64(&mut bcx, jval);
                bcx.def_var(v, val);
                ip += 3;
            }
            Opcode::Pop => {
                if stack.pop().is_none() {
                    return false;
                }
                ip += 1;
            }
            Opcode::Dup => {
                let top = match stack.last() {
                    Some(x) => *x,
                    None => return false,
                };
                stack.push(top);
                ip += 1;
            }
            Opcode::Nop | Opcode::EnterBlock | Opcode::ExitBlock | Opcode::LoopVarsEnd => ip += 1,
            Opcode::LoopVarsBegin => ip += 3,
            Opcode::Jump => {
                let off = match peek_u16(code, ip + 1) {
                    Some(o) => o as i16 as isize,
                    None => return false,
                };
                let t = ((ip + 3) as isize + off).max(0) as usize;
                let blk = match target_block(t) {
                    Some(b) => b,
                    None => return false,
                };
                if !stack.is_empty() {
                    return false;
                }
                bcx.ins().jump(blk, &[]);
                terminated = true;
                ip += 3;
            }
            Opcode::JumpBack => {
                let dist = match peek_u16(code, ip + 1) {
                    Some(d) => d as usize,
                    None => return false,
                };
                let t = match (ip + 3).checked_sub(dist) {
                    Some(t) => t,
                    None => return false,
                };
                let blk = match blocks.get(&t) {
                    Some(&b) => b,
                    None => return false,
                };
                if !stack.is_empty() {
                    return false;
                }
                bcx.ins().jump(blk, &[]);
                terminated = true;
                ip += 3;
            }
            Opcode::JumpIfFalse => {
                let off = match peek_u16(code, ip + 1) {
                    Some(o) => o as i16 as isize,
                    None => return false,
                };
                let cond = match stack.pop() {
                    Some(x) => x,
                    None => return false,
                };
                if !stack.is_empty() {
                    return false;
                }
                let cond = jv_f64(&mut bcx, cond);
                let falsy = falsy_flag(&mut bcx, cond);
                let t = ((ip + 3) as isize + off).max(0) as usize;
                let target = match target_block(t) {
                    Some(b) => b,
                    None => return false,
                };
                let fallthrough = match blocks.get(&(ip + 3)) {
                    Some(&b) => b,
                    None => return false,
                };
                bcx.ins().brif(falsy, target, &[], fallthrough, &[]);
                terminated = true;
                ip += 3;
            }
            Opcode::MathUnary => {
                // #186 — `Math.<fn>(x)`: pop the arg, emit the native op / host call, push the result.
                let id = match peek_u16(code, ip + 1) {
                    Some(v) => v,
                    None => return false,
                };
                let mfn = match MathUnaryFn::from_u16(id) {
                    Some(m) => m,
                    None => return false,
                };
                let x = match stack.pop() {
                    Some(v) => v,
                    None => return false,
                };
                let x = jv_f64(&mut bcx, x);
                let r = emit_math_unary(&mut bcx, math_fref, mfn, x);
                stack.push(JV::f64(r));
                ip += 3;
            }
            _ => match emit_simple_op(&mut bcx, chunk, code, &mut ip, &mut stack, &[], 0) {
                SimpleOp::Handled(_) => {}
                _ => return false, // LoadConst/BinOp/UnaryOp handled; anything else → keep interpreting
            },
        }
    }
    if !terminated {
        return false; // the region must end on its back-edge (a terminator)
    }

    // Exit blocks: flush the live set back through the slots pointer and return the exit id.
    for (&t, &blk) in &exit_blocks {
        bcx.switch_to_block(blk);
        for (p, &slot) in used_slots.iter().enumerate() {
            let v = bcx.use_var(vars[slot as usize]);
            bcx.ins()
                .store(MemFlags::new(), v, slots_ptr, (p * 8) as i32);
        }
        let id = bcx.ins().iconst(types::I32, exit_id[&t] as i64);
        bcx.ins().return_(&[id]);
    }

    bcx.seal_all_blocks();
    bcx.finalize();
    true
}

/// `op_size` of the opcode at `off`, or `None` if the byte is not a known opcode.
fn op_size_at(code: &[u8], off: usize) -> Option<usize> {
    op_size(Opcode::from_u8(*code.get(off)?)?)
}

/// Lower `Math.<mfn>(x)` (#186): a single native op for the ones bit-identical to Rust's `f64`
/// methods (== the `tishlang_builtins::math` fns), else a call to the imported `tish_math_call` host
/// fn (`math_fref`) which routes through [`MathUnaryFn::apply`] — so JIT ≡ VM ≡ interp exactly.
fn emit_math_unary(
    bcx: &mut FunctionBuilder,
    math_fref: cranelift::codegen::ir::FuncRef,
    mfn: MathUnaryFn,
    x: ClifValue,
) -> ClifValue {
    match mfn {
        MathUnaryFn::Sqrt => bcx.ins().sqrt(x),
        MathUnaryFn::Floor => bcx.ins().floor(x),
        MathUnaryFn::Ceil => bcx.ins().ceil(x),
        MathUnaryFn::Trunc => bcx.ins().trunc(x),
        MathUnaryFn::Abs => bcx.ins().fabs(x),
        // `Round` (JS `-0` tie edge), `Sign`, and every transcendental go through the host call so
        // the result is byte-identical to the interpreter (no native-op divergence).
        _ => {
            let id = bcx.ins().iconst(types::I32, mfn as i64);
            let call = bcx.ins().call(math_fref, &[id, x]);
            bcx.inst_results(call)[0]
        }
    }
}

/// Lower `f64` comparison to `1.0`/`0.0` (JS boolean-in-number form).
fn fcmp_f64(bcx: &mut FunctionBuilder, cc: FloatCC, a: ClifValue, b: ClifValue) -> ClifValue {
    let cond = bcx.ins().fcmp(cc, a, b);
    let one = bcx.ins().f64const(1.0);
    let zero = bcx.ins().f64const(0.0);
    bcx.ins().select(cond, one, zero)
}

/// #168 — which Cranelift representation a JIT stack slot currently holds.
///
/// `F64`: an f64 number. `Bool`: an f64 constrained to 0.0/1.0 with JS-boolean semantics (the
/// old `is_bool` flag — comparisons/`!` produce it, `LoadConst Bool` pushes it). `I32`: an i32
/// holding `ToInt32` bits (signed→f64 on materialize). `U32`: an i32 holding `ToUint32` bits
/// (UNSIGNED→f64 on materialize — `>>>` results past 2³¹ stay positive numbers).
///
/// The integer reprs are the point: a bitwise/shift chain (`h = ((h<<13)|(h>>>19)) >>> 0`) used
/// to pay `int→f64→int` conversion ROUND-TRIPS between every op, because the stack could only
/// say "f64 or bool". Now each such op consumes raw int bits via [`jv_i32_bits`] (an identity
/// for `I32`/`U32`) and pushes an int repr; f64 materialization happens once, at a genuine
/// boundary (store/return/float-arith/compare/call), via [`jv_f64`]. `ToInt32` and `ToUint32`
/// share bit patterns, so `I32` vs `U32` only matters at the f64 boundary (signed vs unsigned
/// convert) — the bits themselves are interchangeable as shift/bitwise inputs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Repr {
    F64,
    Bool,
    I32,
    U32,
}

/// A typed JIT stack value: the Cranelift SSA value plus its current [`Repr`].
#[derive(Clone, Copy)]
struct JV {
    v: ClifValue,
    repr: Repr,
}

impl JV {
    fn f64(v: ClifValue) -> Self {
        Self { v, repr: Repr::F64 }
    }
    fn boolean(v: ClifValue) -> Self {
        Self { v, repr: Repr::Bool }
    }
    fn int32(v: ClifValue) -> Self {
        Self { v, repr: Repr::I32 }
    }
    fn uint32(v: ClifValue) -> Self {
        Self { v, repr: Repr::U32 }
    }
    fn is_bool(&self) -> bool {
        self.repr == Repr::Bool
    }
}

/// Materialize a [`JV`] as an f64 — identity for `F64`/`Bool` (a Bool already IS an f64 0/1),
/// one signed/unsigned convert for the integer reprs.
fn jv_f64(bcx: &mut FunctionBuilder, jv: JV) -> ClifValue {
    match jv.repr {
        Repr::F64 | Repr::Bool => jv.v,
        Repr::I32 => bcx.ins().fcvt_from_sint(types::F64, jv.v),
        Repr::U32 => bcx.ins().fcvt_from_uint(types::F64, jv.v),
    }
}

/// Raw `ToInt32` bit pattern of a [`JV`] as an i32 — an identity (zero instructions) for
/// `I32`/`U32` (they share bit patterns), [`js_to_int32`] for the f64 reprs (a Bool's 0.0/1.0
/// converts to 0/1 exactly).
fn jv_i32_bits(bcx: &mut FunctionBuilder, jv: JV) -> ClifValue {
    match jv.repr {
        Repr::I32 | Repr::U32 => jv.v,
        Repr::F64 | Repr::Bool => js_to_int32(bcx, jv.v),
    }
}

/// f64 → JS `ToInt32` as an `I32` clif value, matching `tishlang_core::to_int32` (so a JIT-compiled
/// `& | ^ ~` agrees with the VM fallback). Saturating-cast→`ireduce` is the modulo-2³² for finite
/// values and already gives 0 for NaN / `-∞`; the branchless `select` on `|x| < ∞` maps `+∞` (which
/// saturates to `i64::MAX` → `-1`, the one wrong case) to 0. Branchless, so the hot path stays fast.
fn js_to_int32(bcx: &mut FunctionBuilder, x: ClifValue) -> ClifValue {
    let sat = bcx.ins().fcvt_to_sint_sat(types::I64, x);
    let red = bcx.ins().ireduce(types::I32, sat);
    let absx = bcx.ins().fabs(x);
    let inf = bcx.ins().f64const(f64::INFINITY);
    let finite = bcx.ins().fcmp(FloatCC::LessThan, absx, inf);
    let zero = bcx.ins().iconst(types::I32, 0);
    bcx.ins().select(finite, red, zero)
}

/// #187: import every registered directly-callable callee this chunk references (via `LoadVar name`)
/// into `func`, returning `name → (FuncRef, arity)` for the Call-lowering peephole. Empty when the
/// chunk calls no registered callee (the common case) — then `LoadVar`/`Call` still bail to interp.
#[cfg(not(target_arch = "wasm32"))]
fn build_resolved_callees(
    g: &mut JitGlobal,
    chunk: &Chunk,
    func: &mut cranelift::codegen::ir::Function,
) -> HashMap<Arc<str>, (cranelift::codegen::ir::FuncRef, u8)> {
    let mut resolved = HashMap::new();
    if g.callees.is_empty() {
        return resolved;
    }
    // Collect (name, id, arity) for referenced registered callees first, so the `g.callees` borrow is
    // released before the `g.module` mutable borrow in `declare_func_in_func`.
    let mut refs: Vec<(Arc<str>, cranelift_module::FuncId, u8)> = Vec::new();
    let code = &chunk.code;
    let mut ip = 0usize;
    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => break,
        };
        if op == Opcode::LoadVar {
            if let Some(name) = peek_u16(code, ip + 1).and_then(|ni| chunk.names.get(ni as usize)) {
                if let Some(entry) = g.callees.get(name) {
                    if !refs.iter().any(|(n, _, _)| n == name) {
                        refs.push((Arc::clone(name), entry.id, entry.arity));
                    }
                }
            }
        }
        ip += match op.instruction_size(code, ip) {
            Some(s) => s,
            None => break,
        };
    }
    for (name, id, arity) in refs {
        let fref = g.module.declare_func_in_func(id, func);
        resolved.insert(name, (fref, arity));
    }
    resolved
}

fn compile_chunk(g: &mut JitGlobal, chunk: &Chunk) -> Option<NumericFn> {
    let arity = chunk.param_count as usize;

    // Array-mode (`TISH_JIT_ARRAYS`): if a param is used purely as `arr[i]`/`arr[const]`, compile the
    // 3-pointer array ABI instead of the register-`f64` ABI. mask 0 ⇒ ordinary numeric path below.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (array_mask, writable_mask, bool_write_mask) = if jit_arrays_enabled() {
            classify_params(chunk, arity)
        } else {
            (0, 0, 0)
        };
        if array_mask != 0 {
            return compile_chunk_arrays(
                g,
                chunk,
                arity,
                array_mask,
                writable_mask,
                bool_write_mask,
            );
        }
    }

    // #381: a self-recursive numeric function recurses on the native stack (SelfCall → a native call).
    // Compile it with a trailing `*mut RecurGuard` param so its entry can bail before the stack
    // overflows; non-recursive functions keep the plain register-f64 ABI (no param, no overhead).
    // Gated by `TISH_JIT_RECUR_GUARD` (default ON) so trusted hot-recursion workloads can trade the
    // guard's per-call cost for raw speed — see [`jit_recur_guard_enabled`].
    let recur_guard = jit_recur_guard_enabled() && chunk_has_self_call(chunk);

    // #189: function-local `f64` arrays. Mutually exclusive with `recur_guard` (both would claim the
    // trailing param slot); a self-recursive array function just isn't JV-compiled. `None` from the
    // classifier (an array the JIT can't handle) → empty set → the scan bails on `NewArray` → interpret.
    #[cfg(not(target_arch = "wasm32"))]
    let jv_slots = if !recur_guard && jit_jv_enabled() {
        classify_jv_slots(chunk).unwrap_or_default()
    } else {
        std::collections::HashSet::new()
    };
    #[cfg(target_arch = "wasm32")]
    let jv_slots: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let is_jv = !jv_slots.is_empty();

    let mut sig = g.module.make_signature();
    for _ in 0..arity {
        sig.params.push(AbiParam::new(types::F64));
    }
    if recur_guard {
        sig.params
            .push(AbiParam::new(g.module.target_config().pointer_type())); // *mut RecurGuard
    }
    // #189: a JV function keeps the plain register-`f64` ABI — its deopt flag lives in the per-thread
    // arena (set by `tish_jv_{get,set,deopt}`), not a trailing pointer param.
    sig.returns.push(AbiParam::new(types::F64));

    // Declare the function FIRST so its own `FuncRef` is available while building the body — that is
    // what lets `SelfCall` lower to a native recursive call (the recursion-JIT win). Cranelift
    // resolves the forward self-reference at `finalize_definitions`.
    let name = format!("tish_num_{}", g.counter);
    g.counter += 1;
    let id = match g.module.declare_function(&name, Linkage::Export, &sig) {
        Ok(id) => id,
        Err(_) => return None,
    };

    // Try the loop-capable CFG builder first; if it bails, retry the straight-line/ternary builder.
    // Each attempt needs a fresh function (a partial build leaves the context dirty).
    let mut ctx = g.module.make_context();
    ctx.func.signature = sig.clone();
    let self_ref = g.module.declare_func_in_func(id, &mut ctx.func);
    let math_fref = g.module.declare_func_in_func(g.math_call_id, &mut ctx.func);
    // #189: import the `tish_jv_*` FuncRefs into this function when it has local arrays.
    let jv_ctx = if is_jv {
        Some(JvCtx {
            new: g.module.declare_func_in_func(g.jv_fns.new, &mut ctx.func),
            push: g.module.declare_func_in_func(g.jv_fns.push, &mut ctx.func),
            get: g.module.declare_func_in_func(g.jv_fns.get, &mut ctx.func),
            set: g.module.declare_func_in_func(g.jv_fns.set, &mut ctx.func),
            len: g.module.declare_func_in_func(g.jv_fns.len, &mut ctx.func),
            free: g.module.declare_func_in_func(g.jv_fns.free, &mut ctx.func),
            deopt: g.module.declare_func_in_func(g.jv_fns.deopt, &mut ctx.func),
            slots: &jv_slots,
        })
    } else {
        None
    };
    // #187: import any directly-callable numeric callees this chunk references, so `name(args)` can
    // lower to a native call. Empty for the vast majority of functions.
    let resolved = build_resolved_callees(g, chunk, &mut ctx.func);
    let mut fbctx = FunctionBuilderContext::new();
    let result_bool = match build_body_cfg(
        &mut ctx.func,
        &mut fbctx,
        chunk,
        arity,
        Some(self_ref),
        0,
        recur_guard,
        math_fref,
        jv_ctx.as_ref(),
        &resolved,
    ) {
        Some(b) => b,
        // A JV function has no straight-line fallback (its ABI has the deopt param `build_body`
        // doesn't emit), so a `build_body_cfg` bail means "don't JIT" → interpret.
        None if is_jv => {
            g.module.clear_context(&mut ctx);
            return None;
        }
        None => {
            g.module.clear_context(&mut ctx);
            ctx = g.module.make_context();
            ctx.func.signature = sig.clone();
            fbctx = FunctionBuilderContext::new();
            // build_body (straight-line/ternary) has no self-call path; it bails on SelfCall → VM.
            match build_body(&mut ctx.func, &mut fbctx, chunk, arity) {
                Some(b) => b,
                None => {
                    g.module.clear_context(&mut ctx);
                    return None;
                }
            }
        }
    };

    if g.module.define_function(id, &mut ctx).is_err() {
        g.module.clear_context(&mut ctx);
        return None;
    }
    g.module.clear_context(&mut ctx);
    if g.module.finalize_definitions().is_err() {
        return None;
    }
    let ptr = g.module.get_finalized_function(id);
    // #187: a plain register-`f64` top-level function (no arrays / recursion-guard / bool result) with a
    // provably-stable global name becomes a directly-callable callee. `id` is already finalized above,
    // so a later caller can `declare_func_in_func(id)` and call it. Re-registering a name (a different
    // program reusing the address) overwrites with the new `id`/`fp`; callers resolve against the live
    // registry (and cross-call callers skip the cache), so a stale callee is never invoked.
    if !is_jv && !recur_guard && !result_bool {
        if let Some(name) = &chunk.global_name {
            g.callees.insert(
                Arc::clone(name),
                CalleeEntry {
                    id,
                    arity: arity as u8,
                },
            );
        }
    }
    Some(NumericFn {
        ptr: ptr as usize,
        arity: arity as u8,
        result_bool,
        array_param_mask: 0,
        array_writable_mask: 0,
        array_bool_write_mask: 0,
        recur_guarded: recur_guard,
        jv: is_jv,
        uses_xcall: !resolved.is_empty(),
        void_result: false, // register-f64 functions return a real f64
    })
}

/// Classify each param slot of an array-mode candidate. Returns `(array_mask, writable_mask,
/// bool_write_mask)`: **array bit k ⇒ param k is an ARRAY** used only as `arr[i]` (read, `GetIndex`) or
/// `arr[i] = v` (write, `SetIndex`); **writable bit k ⇒ that array is written**; **bool_write bit k ⇒ a
/// writable array written with bool consts (`arr[i]=true`)**. Returns `(0, 0, 0)` when there are no
/// array params OR the function is ineligible (a param used as both an array and a number, a mix of bool
/// and non-bool writes to one array, an index shape the peephole can't consume, etc.). A `0` mask is
/// always safe: the caller takes the ordinary
/// numeric path (which itself bails on `GetIndex`/`SetIndex`), and `try_call_array_jit` re-checks every
/// arg's runtime type, so a misclassification bails to the interpreter rather than miscompiling.
#[cfg(not(target_arch = "wasm32"))]
fn classify_params(chunk: &Chunk, arity: usize) -> (u8, u8, u8) {
    if arity == 0 || arity > 8 || (chunk.num_slots as usize) == 0 {
        return (0, 0, 0);
    }
    let code = &chunk.code;
    // #187: which slots hold a Bool value (`arr[i] = boolLocal` must be classified a BOOL write, not a
    // number write, so a bool local stored into a NUMBER array is caught by the runtime type guard).
    // Computed unconditionally — a superset only makes the guard bail more (always sound), never miscompile.
    let bool_slots = classify_bool_slots(chunk);
    let mut stored = [false; 8]; // param REASSIGNED via StoreLocal (an array param can't be — bail)
    let mut used_array = [false; 8];
    let mut written = [false; 8];
    // #187: per writable array param, was it written with a Bool const (`arr[i]=true`) and/or a
    // non-bool value (`arr[i]=5`)? The JIT flattens every element to f64, so the writeback must re-box
    // by the ORIGINAL element type; that is only sound when the whole array stays one type. A mix of
    // bool and non-bool writes to the SAME array can never be re-boxed uniformly ⇒ bail at compile time.
    // `wrote_bool` also feeds the runtime guard (write-kind must match the entry array's element type).
    let mut wrote_bool = [false; 8];
    let mut wrote_nonbool = [false; 8];
    let mut ip = 0usize;
    let simple = |o: Option<Opcode>| matches!(o, Some(Opcode::LoadLocal) | Some(Opcode::LoadConst));
    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => return (0, 0, 0),
        };
        let size = match op_size(op) {
            Some(s) => s,
            // #187: a bare `LoadVar name` / `Call argc` (a global function call whose result is numeric)
            // doesn't touch param classification — treat as a sized no-op. `build_body_cfg` is the gate:
            // it lowers the call only if the callee resolves, else the whole function bails to interp.
            None if matches!(op, Opcode::LoadVar | Opcode::Call) => 3,
            None => return (0, 0, 0), // an opcode the array CFG can't handle ⇒ ineligible
        };
        match op {
            Opcode::LoadLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return (0, 0, 0),
                };
                let at = |off: usize| code.get(ip + off).copied().and_then(Opcode::from_u8);
                // Read peephole: `LoadLocal(arr) ; (LoadLocal|LoadConst) idx ; GetIndex`.
                if slot < arity && simple(at(3)) && at(6) == Some(Opcode::GetIndex) {
                    used_array[slot] = true;
                    ip += 7;
                    continue;
                }
                // Write peephole: `LoadLocal(arr) ; (LoadLocal|LoadConst) idx ; (LoadLocal|LoadConst)
                // val ; Dup ; SetIndex` — a plain `arr[i] = v` with a simple index and value.
                if slot < arity
                    && simple(at(3))
                    && simple(at(6))
                    && at(9) == Some(Opcode::Dup)
                    && at(10) == Some(Opcode::SetIndex)
                {
                    used_array[slot] = true;
                    written[slot] = true;
                    // Classify the written VALUE (operand at offset 6) so the runtime writeback re-boxes
                    // by the right element type. Only a `LoadConst(Bool)` is a *definite* bool write.
                    // A `LoadLocal(slot)` is trickier: `classify_bool_slots` is a SUPERSET (insert-only —
                    // it never un-tags a slot that is later reassigned a number, and `build_body_cfg`
                    // permits `b = n` into a bool-tagged slot), so a bool-tagged slot may actually hold a
                    // NUMBER at the write site. We can't tell the runtime kind statically, and the flat
                    // `f64` writeback would re-box it wrong — so bail the whole function. (Numeric-slot
                    // writes — e.g. spectral_norm's `out[i] = sum` — are fine: not bool-tagged, non-bool.)
                    let val_is_bool = match at(6) {
                        Some(Opcode::LoadConst) => matches!(
                            peek_u16(code, ip + 7).and_then(|ci| chunk.constants.get(ci as usize)),
                            Some(Constant::Bool(_))
                        ),
                        Some(Opcode::LoadLocal)
                            if peek_u16(code, ip + 7)
                                .is_some_and(|s| bool_slots.contains(&(s as usize))) =>
                        {
                            return (0, 0, 0);
                        }
                        _ => false,
                    };
                    if val_is_bool {
                        wrote_bool[slot] = true;
                    } else {
                        wrote_nonbool[slot] = true;
                    }
                    ip += 11;
                    continue;
                }
                // Otherwise a bare `LoadLocal(param)` — either a numeric value (fine; a param is an
                // array iff INDEXED) or a self-recursive array-call argument (build_body_cfg validates +
                // consumes it, else bails). Neither affects classification.
            }
            Opcode::StoreLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return (0, 0, 0),
                };
                if slot < arity {
                    stored[slot] = true; // reassigning the binding — an array param can't be (see tail)
                }
            }
            // Any `GetIndex`/`SetIndex` not consumed by a peephole above ⇒ an index/value shape we don't
            // handle (`arr[i+1]`, `arr[i] = a + b`, `arr[brr[i]]`, …) ⇒ ineligible.
            Opcode::GetIndex | Opcode::SetIndex => return (0, 0, 0),
            _ => {}
        }
        ip += size;
    }
    let mut array_mask = 0u8;
    let mut writable_mask = 0u8;
    let mut bool_write_mask = 0u8;
    for k in 0..arity {
        if used_array[k] {
            // A REASSIGNED array param (`arr = …`) would break the handle-reuse assumption (esp. the
            // self-call reusing `handles_ptr`), so bail the whole function. A `arr[i] = v` element write
            // is `written`, not `stored`, and stays fine.
            if stored[k] {
                return (0, 0, 0);
            }
            // A single array written with BOTH bool and non-bool values can never be re-boxed uniformly
            // (the JIT has already flattened everything to f64) ⇒ bail rather than miscompile.
            if wrote_bool[k] && wrote_nonbool[k] {
                return (0, 0, 0);
            }
            array_mask |= 1u8 << k;
            if written[k] {
                writable_mask |= 1u8 << k;
                if wrote_bool[k] {
                    bool_write_mask |= 1u8 << k;
                }
            }
        }
    }
    (array_mask, writable_mask, bool_write_mask)
}

/// Compile an array-mode function: numeric params + array params (read as `arr[i]`). Uses ONE uniform
/// ABI for every such function — `extern "C" fn(numeric: *const f64, handles: *const ArrayHandle,
/// deopt: *mut u8) -> f64` — so there is a single transmute (no per-arity explosion). Out-of-bounds
/// reads set `*deopt` and bail (the caller re-runs the interpreter); non-numeric arrays never reach
/// here (the VM wrapper only calls this when every element is a `Value::Number`).
#[cfg(not(target_arch = "wasm32"))]
fn compile_chunk_arrays(
    g: &mut JitGlobal,
    chunk: &Chunk,
    arity: usize,
    mask: u8,
    writable_mask: u8,
    bool_write_mask: u8,
) -> Option<NumericFn> {
    // #187: a self-recursive array-mode function (queens' `place`) recurses on the native stack, so —
    // like #381's register-f64 recursion — it gets a trailing `*mut RecurGuard` param + an entry
    // SP-bail, keeping unbounded recursion a catchable RangeError instead of a SIGSEGV.
    let recursive = chunk_has_self_call(chunk) && jit_recur_guard_enabled();
    let ptr_ty = g.module.target_config().pointer_type();
    let mut sig = g.module.make_signature();
    sig.params.push(AbiParam::new(ptr_ty)); // numeric_ptr
    sig.params.push(AbiParam::new(ptr_ty)); // handles_ptr
    sig.params.push(AbiParam::new(ptr_ty)); // deopt_ptr
    if recursive {
        sig.params.push(AbiParam::new(ptr_ty)); // *mut RecurGuard (#187/#381)
    }
    sig.returns.push(AbiParam::new(types::F64));

    let name = format!("tish_arr_{}", g.counter);
    g.counter += 1;
    let id = g
        .module
        .declare_function(&name, Linkage::Export, &sig)
        .ok()?;

    let mut ctx = g.module.make_context();
    ctx.func.signature = sig.clone();
    // #187: declare this function's own FuncRef so a self-recursive array-mode call (queens' `place`
    // passing `cols`/`diag1`/`diag2` back to itself) lowers to a native call reusing the same
    // `handles_ptr`/`deopt_ptr` — only the numeric args are re-marshalled per level.
    let self_ref = g.module.declare_func_in_func(id, &mut ctx.func);
    let math_fref = g.module.declare_func_in_func(g.math_call_id, &mut ctx.func);
    // #187: array-mode functions (e.g. spectral_norm's multiplyAv) may call a register-f64 callee.
    let resolved = build_resolved_callees(g, chunk, &mut ctx.func);
    let mut fbctx = FunctionBuilderContext::new();
    // No JV local arrays in array-param mode → pass None for the JV context.
    if build_body_cfg(
        &mut ctx.func,
        &mut fbctx,
        chunk,
        arity,
        Some(self_ref),
        mask,
        recursive, // #187: array-mode recursion guard (the entry SP-bail keys off this)
        math_fref,
        None,
        &resolved,
    )
    .is_none()
    {
        g.module.clear_context(&mut ctx);
        return None;
    }
    if g.module.define_function(id, &mut ctx).is_err() {
        g.module.clear_context(&mut ctx);
        return None;
    }
    g.module.clear_context(&mut ctx);
    if g.module.finalize_definitions().is_err() {
        return None;
    }
    let ptr = g.module.get_finalized_function(id);
    Some(NumericFn {
        ptr: ptr as usize,
        arity: arity as u8,
        result_bool: false,
        array_param_mask: mask,
        array_writable_mask: writable_mask,
        array_bool_write_mask: bool_write_mask,
        recur_guarded: recursive, // #187: array-mode fn has a trailing RecurGuard param → call_arrays passes it
        jv: false,
        uses_xcall: !resolved.is_empty(),
        void_result: chunk_is_void(chunk),
    })
}

/// Outcome of trying to emit one *straight-line* numeric opcode.
enum SimpleOp {
    /// Handled: bytecode consumed, IR emitted, `bool` flags a comparison/`!` result.
    #[allow(dead_code)]
    // reserved: the flag will carry a comparison/`!` result; currently always false
    Handled(bool),
    /// The opcode is control flow / `Return` / non-numeric — NOT consumed; caller decides.
    NotSimple,
    /// A simple-op *type* but an unsupported variant (Pow / shift / non-number const) —
    /// the whole function is ineligible. State may be partially consumed; caller bails.
    Unsupported,
}

/// Emit one straight-line numeric opcode at `*ip` (no control flow, no `Return`). On
/// `NotSimple` neither `ip` nor `stack` is touched, so the caller can re-dispatch.
fn emit_simple_op(
    bcx: &mut FunctionBuilder,
    chunk: &Chunk,
    code: &[u8],
    ip: &mut usize,
    stack: &mut Vec<JV>,
    params: &[ClifValue],
    arity: usize,
) -> SimpleOp {
    let op = match code.get(*ip).copied().and_then(Opcode::from_u8) {
        Some(o) => o,
        None => return SimpleOp::NotSimple,
    };
    match op {
        Opcode::Nop | Opcode::LoadLocal | Opcode::LoadConst | Opcode::BinOp | Opcode::UnaryOp => {}
        // Control flow / member / index / call / array / object / Return → caller handles.
        _ => return SimpleOp::NotSimple,
    }
    *ip += 1;
    match op {
        Opcode::Nop => {}
        Opcode::LoadLocal => {
            let slot = match read_u16(code, ip) {
                Some(s) => s as usize,
                None => return SimpleOp::Unsupported,
            };
            // Straight-line simple fns declare no locals; only params (numbers).
            if slot >= arity {
                return SimpleOp::Unsupported;
            }
            stack.push(JV::f64(params[slot]));
        }
        Opcode::LoadConst => {
            let idx = match read_u16(code, ip) {
                Some(i) => i as usize,
                None => return SimpleOp::Unsupported,
            };
            match chunk.constants.get(idx) {
                Some(Constant::Number(n)) => {
                    let v = bcx.ins().f64const(*n);
                    stack.push(JV::f64(v));
                }
                Some(Constant::Bool(b)) => {
                    let v = bcx.ins().f64const(if *b { 1.0 } else { 0.0 });
                    stack.push(JV::boolean(v));
                }
                _ => return SimpleOp::Unsupported,
            }
        }
        Opcode::BinOp => {
            let bop = match read_u16(code, ip).map(|r| r as u8).and_then(u8_to_binop) {
                Some(b) => b,
                None => return SimpleOp::Unsupported,
            };
            if stack.len() < 2 {
                return SimpleOp::Unsupported;
            }
            let r = stack.pop().unwrap();
            let l = stack.pop().unwrap();
            // #187: a bool value (from a bool slot or a `LoadConst Bool`) can't take part in an
            // EQUALITY compare here — the JIT compares the `f64` 0/1 bits, but JS `===`/`!==` (and
            // strict `==`/`!=`) treat `0 === false` as FALSE across types. Bail so the interpreter
            // decides. Relational (`<`/`>`/…) coerces bool→0/1 in both, so those stay JIT'd.
            // Integer reprs are plain NUMBERS, so they take part in every compare.
            let is_eq = matches!(
                bop,
                BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe
            );
            if is_eq && (l.is_bool() || r.is_bool()) {
                return SimpleOp::Unsupported;
            }
            // #168: float arithmetic / comparisons materialize both operands as f64 at this
            // boundary; bitwise/shift ops consume raw int bits (identity for I32/U32 operands)
            // and PUSH an integer repr — a chained `((h<<13)|(h>>>19))>>>0` stays in i32
            // registers with zero intermediate converts. `Mul` stays f64 ON PURPOSE: V8 rounds
            // `h * K` past 2^53 the same way, and the gauntlet checksum pins that agreement.
            let v: JV = match bop {
                BinOp::Add => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::f64(bcx.ins().fadd(lf, rf))
                }
                BinOp::Sub => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::f64(bcx.ins().fsub(lf, rf))
                }
                BinOp::Mul => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::f64(bcx.ins().fmul(lf, rf))
                }
                BinOp::Div => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::f64(bcx.ins().fdiv(lf, rf))
                }
                BinOp::Eq | BinOp::StrictEq => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::boolean(fcmp_f64(bcx, FloatCC::Equal, lf, rf))
                }
                BinOp::Ne | BinOp::StrictNe => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::boolean(fcmp_f64(bcx, FloatCC::NotEqual, lf, rf))
                }
                BinOp::Lt => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::boolean(fcmp_f64(bcx, FloatCC::LessThan, lf, rf))
                }
                BinOp::Le => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::boolean(fcmp_f64(bcx, FloatCC::LessThanOrEqual, lf, rf))
                }
                BinOp::Gt => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::boolean(fcmp_f64(bcx, FloatCC::GreaterThan, lf, rf))
                }
                BinOp::Ge => {
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    JV::boolean(fcmp_f64(bcx, FloatCC::GreaterThanOrEqual, lf, rf))
                }
                BinOp::Mod => {
                    // f64 remainder a - trunc(a/b)*b — exactly Rust's `%`, which the
                    // VM's eval_binop uses, so JIT and VM-fallback agree bit-for-bit.
                    let (lf, rf) = (jv_f64(bcx, l), jv_f64(bcx, r));
                    let q = bcx.ins().fdiv(lf, rf);
                    let t = bcx.ins().trunc(q);
                    let p = bcx.ins().fmul(t, rf);
                    JV::f64(bcx.ins().fsub(lf, p))
                }
                // Bitwise AND/OR/XOR via JS ToInt32 (modulo 2³², NaN/±∞ → 0) — [`jv_i32_bits`]
                // is [`js_to_int32`] for f64 operands and an IDENTITY for int-repr operands.
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    let li = jv_i32_bits(bcx, l);
                    let ri = jv_i32_bits(bcx, r);
                    let res = match bop {
                        BinOp::BitAnd => bcx.ins().band(li, ri),
                        BinOp::BitOr => bcx.ins().bor(li, ri),
                        BinOp::BitXor => bcx.ins().bxor(li, ri),
                        _ => unreachable!(),
                    };
                    JV::int32(res)
                }
                // Shifts. JS masks the count to the low 5 bits (`& 31`); the low 5 bits of
                // `ToInt32(r)` equal `ToUint32(r)`, so `jv_i32_bits(r)` carries the right amount.
                // We mask explicitly (`& 31`) so correctness never depends on Cranelift's own
                // amount-masking convention. `<<`/`>>` are signed-domain (I32 repr — signed→f64
                // when materialized); `>>>` is logical on the unsigned bits (U32 repr — an
                // UNSIGNED→f64 materialize keeps a bit-31 result a positive number, JS `>>>`).
                // Bit-for-bit with vm.rs `eval_binop`: Shl/Shr = `to_int32(l).wrapping_sh*
                // (to_uint32(r))`, UShr = `to_uint32(l).wrapping_shr(to_uint32(r))`.
                BinOp::Shl | BinOp::Shr | BinOp::UShr => {
                    let li = jv_i32_bits(bcx, l);
                    let amt = jv_i32_bits(bcx, r);
                    let mask = bcx.ins().iconst(types::I32, 31);
                    let amt = bcx.ins().band(amt, mask);
                    match bop {
                        BinOp::Shl => JV::int32(bcx.ins().ishl(li, amt)),
                        BinOp::Shr => JV::int32(bcx.ins().sshr(li, amt)),
                        _ => JV::uint32(bcx.ins().ushr(li, amt)),
                    }
                }
                // Pow/In/And/Or: fall back to the VM.
                _ => return SimpleOp::Unsupported,
            };
            stack.push(v);
        }
        Opcode::UnaryOp => {
            let uop = match read_u16(code, ip).map(|r| r as u8).and_then(u8_to_unaryop) {
                Some(u) => u,
                None => return SimpleOp::Unsupported,
            };
            let o = match stack.pop() {
                Some(x) => x,
                None => return SimpleOp::Unsupported,
            };
            let v: JV = match uop {
                UnaryOp::Neg => {
                    let of = jv_f64(bcx, o);
                    JV::f64(bcx.ins().fneg(of))
                }
                UnaryOp::Pos => JV::f64(jv_f64(bcx, o)),
                UnaryOp::Not => {
                    let of = jv_f64(bcx, o);
                    let zero = bcx.ins().f64const(0.0);
                    JV::boolean(fcmp_f64(bcx, FloatCC::Equal, of, zero))
                }
                // `~x` = `!ToInt32(x)` — JS ToInt32 (modulo, NaN/±∞ → 0) via [`jv_i32_bits`]
                // (identity for an int-repr operand), matching the VM so a JIT-compiled `~`
                // can't diverge on large/non-finite values. Pushes I32 — a `~` chain stays
                // in integer registers.
                UnaryOp::BitNot => {
                    let oi = jv_i32_bits(bcx, o);
                    JV::int32(bcx.ins().bnot(oi))
                }
                _ => return SimpleOp::Unsupported,
            };
            stack.push(v);
        }
        _ => unreachable!("guarded above"),
    }
    SimpleOp::Handled(false)
}

/// `is_truthy(cond)` as a Cranelift bool, matching the VM: a number is truthy iff it is
/// nonzero AND not NaN. Returns the **falsy** flag (so callers can `select(falsy, else, then)`
/// without a logical-not, which `bnot` can't express on a 0/1 value).
fn falsy_flag(bcx: &mut FunctionBuilder, cond: ClifValue) -> ClifValue {
    let zero = bcx.ins().f64const(0.0);
    let eq_zero = bcx.ins().fcmp(FloatCC::Equal, cond, zero); // ordered: false for NaN
    let is_nan = bcx.ins().fcmp(FloatCC::NotEqual, cond, cond); // UNE self-compare: true iff NaN
    bcx.ins().bor(eq_zero, is_nan)
}

/// Translate the chunk's numeric bytecode into the function body. Straight-line ops plus the
/// **ternary `cond ? A : B`** pattern (forward `JumpIfFalse`/`Jump` with branch-free, net-+1,
/// agreeing-`is_bool` arms) → a Cranelift `select`. Loops (`JumpBack`), early returns inside a
/// branch, nested branches, calls, member/index, or mismatched `is_bool` all return `None` so the
/// VM runs the chunk instead — purely additive. Returns `Some(result_is_bool)`.
/// Byte size of an opcode the loop-JIT understands; `None` ⇒ unsupported (bail → VM).
fn op_size(op: Opcode) -> Option<usize> {
    use Opcode::*;
    Some(match op {
        Nop | Pop | Dup | Return | LoopVarsEnd | EnterBlock | ExitBlock | GetIndex | SetIndex => 1,
        LoadLocal | StoreLocal | LoadConst | BinOp | UnaryOp | Jump | JumpIfFalse | JumpBack
        | LoopVarsBegin | SelfCall | MathUnary => 3,
        _ => return None,
    })
}

/// Cheap pre-scan: does this chunk contain a `SelfCall`? Decided in `compile_chunk` (before the sig is
/// built) so a self-recursive function gets the `RecurGuard` param. Walks by `op_size`; if a size is
/// unknown it returns `false`, which is safe: `build_body_cfg` uses the same `op_size` and would itself
/// bail on that opcode, so the function isn't JIT'd (it runs on the guarded VM instead). #381
fn chunk_has_self_call(chunk: &Chunk) -> bool {
    let code = &chunk.code;
    let mut ip = 0;
    while ip < code.len() {
        let Some(op) = Opcode::from_u8(code[ip]) else {
            return false;
        };
        if op == Opcode::SelfCall {
            return true;
        }
        let Some(size) = op_size(op) else {
            return false;
        };
        ip += size;
    }
    false
}

/// #189 — identify function-LOCAL slots that hold non-escaping `f64` arrays (JV slots). A slot
/// qualifies iff its ONLY definition is an empty array literal (`NewArray 0` immediately followed by
/// `StoreLocal slot`) and it is never stored from anything else (which would alias/escape it). Any
/// `NewArray` with a non-empty literal, or one not immediately stored to a slot, bails the WHOLE
/// function (`None`) — such an array can't be JV-compiled and the function has array opcodes the JIT
/// can't handle anyway. `Some(empty)` = "no local arrays" (the normal numeric path).
///
/// This is a cheap pass over STORES only; the [`build_body_cfg`] emission validates USES by bailing
/// #187: slots that ever receive a BOOLEAN value, so the numeric JIT can represent them as `f64`
/// `0.0`/`1.0` instead of bailing at `StoreLocal`. Purely syntactic: a slot is tagged when a
/// `StoreLocal(slot)` is immediately preceded by an op that pushes a bool — `LoadConst(Bool)`, a
/// comparison `BinOp` (`==`/`!=`/`===`/`!==`/`<`/`<=`/`>`/`>=`), or `UnaryOp !`. Over-tagging is safe:
/// a tagged slot only makes its `LoadLocal`s carry `is_bool`, which at worst forces a diverging
/// `bool === number` compare or a `return bool` to bail to the interpreter (never a miscompile). A
/// bytecode decode failure stops the scan early (→ fewer tags → more conservative), never a panic.
/// #187: is this a VOID function — one that only ever `return`s the implicit `null` (a side-effect
/// function like `multiplyAv`, which writes an array param and falls through)? True iff EVERY `Return`
/// is immediately preceded by `LoadConst Null`. The array-mode JIT can then emit a dummy `f64` result
/// for such a function and its wrapper returns `Value::Null` (matching the interpreter). Conservative:
/// a decode failure or any value-returning path makes it `false`.
#[cfg(not(target_arch = "wasm32"))]
fn chunk_is_void(chunk: &Chunk) -> bool {
    let code = &chunk.code;
    let mut ip = 0usize;
    let mut prev_null = false;
    let mut saw_return = false;
    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => return false,
        };
        if op == Opcode::Return {
            saw_return = true;
            if !prev_null {
                return false; // a value-returning path ⇒ not void
            }
        }
        prev_null = op == Opcode::LoadConst
            && matches!(
                peek_u16(code, ip + 1).and_then(|i| chunk.constants.get(i as usize)),
                Some(Constant::Null)
            );
        ip += match op.instruction_size(code, ip) {
            Some(s) => s,
            None => return false,
        };
    }
    saw_return
}

fn classify_bool_slots(chunk: &Chunk) -> std::collections::HashSet<usize> {
    let code = &chunk.code;
    let mut set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut ip = 0usize;
    let mut prev_pushes_bool = false;
    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => break,
        };
        let size = match op.instruction_size(code, ip) {
            Some(s) => s,
            None => break,
        };
        if op == Opcode::StoreLocal && prev_pushes_bool {
            if let Some(s) = peek_u16(code, ip + 1) {
                set.insert(s as usize);
            }
        }
        prev_pushes_bool = match op {
            Opcode::LoadConst => matches!(
                peek_u16(code, ip + 1).and_then(|i| chunk.constants.get(i as usize)),
                Some(Constant::Bool(_))
            ),
            Opcode::BinOp => matches!(
                peek_u16(code, ip + 1)
                    .map(|r| r as u8)
                    .and_then(u8_to_binop),
                Some(
                    BinOp::Eq
                        | BinOp::Ne
                        | BinOp::StrictEq
                        | BinOp::StrictNe
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                )
            ),
            Opcode::UnaryOp => matches!(
                peek_u16(code, ip + 1)
                    .map(|r| r as u8)
                    .and_then(u8_to_unaryop),
                Some(UnaryOp::Not)
            ),
            _ => false,
        };
        ip += size;
    }
    set
}

/// on any misuse of a JV ref (it reaching a numeric op, a return, a call arg, a block boundary, …),
/// so a mis-shaped use never miscompiles — it just falls back to the interpreter.
fn classify_jv_slots(chunk: &Chunk) -> Option<std::collections::HashSet<usize>> {
    let code = &chunk.code;
    let mut from_newarray: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut from_other: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut has_newarray = false;
    let mut ip = 0usize;
    let mut prev_newarray0 = false;
    while ip < code.len() {
        let op = Opcode::from_u8(*code.get(ip)?)?;
        let size = op.instruction_size(code, ip)?;
        let is_newarray0 = op == Opcode::NewArray && peek_u16(code, ip + 1)? == 0;
        if op == Opcode::NewArray {
            has_newarray = true;
            if peek_u16(code, ip + 1)? != 0 {
                return None; // non-empty array literal → not JV-compilable
            }
            // An empty `[]` must be stored straight into a slot (`let a = []`); anything else means
            // the fresh array is used inline / escapes → bail.
            if Opcode::from_u8(*code.get(ip + size)?)? != Opcode::StoreLocal {
                return None;
            }
        }
        if op == Opcode::StoreLocal {
            let slot = peek_u16(code, ip + 1)? as usize;
            if prev_newarray0 {
                from_newarray.insert(slot);
            } else {
                from_other.insert(slot);
            }
        }
        prev_newarray0 = is_newarray0;
        ip += size;
    }
    if !has_newarray {
        return Some(std::collections::HashSet::new());
    }
    // A NewArray-defined slot that is ALSO stored otherwise is aliased/polymorphic — bail the whole
    // function (its array can't be JV-compiled, and it has `NewArray` the JIT otherwise rejects).
    if from_newarray.iter().any(|s| from_other.contains(s)) {
        return None;
    }
    Some(from_newarray)
}

/// Read a big-endian u16 operand at `off` without advancing (matches [`read_u16`]).
#[inline]
fn peek_u16(code: &[u8], off: usize) -> Option<u16> {
    let a = *code.get(off)? as u16;
    let b = *code.get(off + 1)? as u16;
    Some((a << 8) | b)
}

/// Read a "simple" operand at byte `at` — a `LoadLocal(slot)` (→ the slot's f64 Variable) or a
/// `LoadConst(Number)` (→ an f64 const) — for the array read/write peepholes. `None` for any other
/// opcode / a non-numeric const, which makes the caller bail. #187
#[cfg(not(target_arch = "wasm32"))]
fn read_simple_operand(
    bcx: &mut FunctionBuilder,
    code: &[u8],
    at: usize,
    chunk: &Chunk,
    vars: &[Variable],
) -> Option<ClifValue> {
    match Opcode::from_u8(*code.get(at)?)? {
        Opcode::LoadLocal => {
            let s = peek_u16(code, at + 1)? as usize;
            Some(bcx.use_var(*vars.get(s)?))
        }
        Opcode::LoadConst => {
            let ci = peek_u16(code, at + 1)? as usize;
            match chunk.constants.get(ci) {
                Some(Constant::Number(n)) => Some(bcx.ins().f64const(*n)),
                // #187: a boolean stored into a (bool) array element — `arr[i] = true` — as `f64` 0/1.
                Some(Constant::Bool(b)) => Some(bcx.ins().f64const(if *b { 1.0 } else { 0.0 })),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Control-flow JIT: lower a slot-based numeric function WITH loops/branches to native code. Uses a
/// cranelift `Variable` per slot (loop-carried locals are mutable across blocks; cranelift inserts the
/// SSA phis at `seal_all_blocks`), and one block per bytecode jump target. Handles LoadLocal/StoreLocal,
/// LoadConst, numeric BinOp/UnaryOp, Pop/Dup/Nop, LoopVarsBegin/End (skipped — a slotted loop var needs
/// no per-iteration overlay), Jump/JumpIfFalse/JumpBack/Return, AND direct calls to JIT-compiled callees
/// (a `LoadVar name_idx` + `Call arity` where the callee has a `NumericFn` in `callees` → emit a
/// direct cranelift `call` to the native code pointer, skipping all Value boxing). CONSERVATIVE: bails
/// (→ caller → VM) on any other opcode, a non-empty operand stack at a block boundary, boolean slots,
/// or a `Call` whose callee is not in `callees`. ADDITIVE: a bail just runs the VM.
/// Returns `Some(false)` (Number result) on success.
#[allow(clippy::too_many_arguments)] // each param is a distinct, documented lowering mode/handle
fn build_body_cfg(
    func: &mut cranelift::codegen::ir::Function,
    fbctx: &mut FunctionBuilderContext,
    chunk: &Chunk,
    arity: usize,
    self_ref: Option<cranelift::codegen::ir::FuncRef>,
    // Array-mode bitmask (bit k ⇒ param k is an array). `0` ⇒ ordinary register-`f64` ABI (every
    // existing caller passes 0, so that path is byte-identical). Nonzero ⇒ the 3-pointer array ABI:
    // params are `[numeric_ptr, handles_ptr, deopt_ptr]` and `arr[i]` lowers to a bounds-checked load.
    array_mask: u8,
    // #381: when true (self-recursive, plain-numeric path only) the signature has a trailing
    // `*mut RecurGuard` param, and the function entry emits a stack-pointer check that bails before
    // the native recursion overflows. `false` for array mode and every non-recursive function.
    recur_guard: bool,
    // #186: the imported `tish_math_call` host fn, for lowering `Math.<fn>` (`MathUnary`) intrinsics.
    math_fref: cranelift::codegen::ir::FuncRef,
    // #189: `Some` when the function has local `f64` arrays — carries the `tish_jv_*` `FuncRef`s + the
    // JV slot set. Those slots become `i64` handle Variables and their array ops lower to `tish_jv_*`
    // calls; an out-of-bounds index sets the per-thread deopt flag the wrapper re-interprets on.
    jv: Option<&JvCtx>,
    // #187: `name → (FuncRef, arity)` for directly-callable numeric callees imported into `func`. A
    // `LoadVar name ; args ; Call` whose name is here lowers to a native call; unresolved names bail.
    resolved: &HashMap<Arc<str>, (cranelift::codegen::ir::FuncRef, u8)>,
) -> Option<bool> {
    let code = &chunk.code;
    let num_slots = chunk.num_slots as usize;
    if num_slots == 0 || num_slots > 256 {
        return None;
    }
    // #187: slots that hold a boolean (represented as `f64` 0/1). A `LoadLocal` of one carries
    // `is_bool` so a diverging `bool === number` compare or a `return bool` bails to the interpreter.
    let bool_slots = if jit_bool_slots_enabled() {
        classify_bool_slots(chunk)
    } else {
        std::collections::HashSet::new()
    };
    // #187: a VOID array-mode function (only returns the implicit `null`, e.g. a side-effect writer)
    // gets a dummy `f64` result; `try_call_array_jit` returns `Value::Null` for it (matches interp).
    let is_void = array_mask != 0 && chunk_is_void(chunk);

    // 1. Validate every opcode is supported + collect block leaders (jump targets, the fall-through
    //    after each branch, entry). Bail on any unsupported opcode (so we never mis-size the scan).
    let mut leaders: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    leaders.insert(0);
    let mut has_loop = false;
    let mut has_self_call = false;
    let mut ip = 0;
    while ip < code.len() {
        let op = Opcode::from_u8(code[ip])?;
        // For a JV function the array opcodes (NewArray/GetMember/GetIndex/SetIndex/Call) are valid
        // and must be sized via the full instruction table; the translate loop then validates each is
        // a real JV op (else it bails). Non-JV functions keep the strict `op_size` whitelist.
        let size = match op_size(op) {
            Some(s) => s,
            None if jv.is_some() => op.instruction_size(code, ip)?,
            // #187: `LoadVar`/`Call` are 3 bytes; the translate loop lowers them to a native call if the
            // callee resolved, else bails. Sizing them here lets a function with a resolvable call pass
            // the scan (an unresolvable one still bails in translation).
            None if matches!(op, Opcode::LoadVar | Opcode::Call) => 3,
            None => return None,
        };
        // A SelfCall whose arity != this function's arity can't be a plain f64-ABI native call → bail.
        if op == Opcode::SelfCall {
            if self_ref.is_none() || peek_u16(code, ip + 1)? as usize != arity {
                return None;
            }
            has_self_call = true;
        }
        match op {
            // A conditional branch: BOTH the target and the fall-through are reachable.
            Opcode::JumpIfFalse => {
                let off = peek_u16(code, ip + 1)? as i16 as isize;
                leaders.insert(((ip + 3) as isize + off).max(0) as usize);
                leaders.insert(ip + 3);
            }
            // Unconditional jumps: only the TARGET is a leader. The byte after the jump is reachable
            // iff something else jumps to it — in which case that jump adds it. Code after an
            // unconditional terminator (Jump/JumpBack/Return) that nothing targets is UNREACHABLE
            // (e.g. the compiler's trailing implicit `LoadConst Null; Return`) and must be skipped,
            // not translated — else we'd bail on `LoadConst Null`.
            Opcode::Jump => {
                let off = peek_u16(code, ip + 1)? as i16 as isize;
                leaders.insert(((ip + 3) as isize + off).max(0) as usize);
            }
            Opcode::JumpBack => {
                let dist = peek_u16(code, ip + 1)? as usize;
                leaders.insert((ip + 3).checked_sub(dist)?);
                has_loop = true;
            }
            _ => {}
        }
        ip += size;
    }
    // Worth the CFG path when there's a loop OR a self-recursive call (e.g. `fib`: branches + early
    // return + recursion, no loop). Pure straight-line/ternary stays on `build_body`.
    if !has_loop && !has_self_call {
        return None;
    }

    let mut bcx = FunctionBuilder::new(func, fbctx);
    let mut blocks: std::collections::BTreeMap<usize, Block> =
        leaders.iter().map(|&o| (o, bcx.create_block())).collect();
    let entry = *blocks.get(&0)?;
    bcx.append_block_params_for_function_params(entry);
    bcx.switch_to_block(entry);
    let params: Vec<ClifValue> = bcx.block_params(entry).to_vec();

    // #381: self-recursive numeric function → `entry` is a stack-pointer bail check (tish's deopt
    // pattern, not V8's throw-from-JIT): compare SP to the caller-provided `stack_limit`; if the
    // recursion has driven SP past it, store `tripped = 1` into the RecurGuard and return a sentinel
    // instead of recursing further. The body then runs in `body_block`; `entry` is the function's real
    // entry point, so this check runs on every (including recursive) call. Loops jumping back to offset
    // 0 land in `body_block` (no re-check needed — a loop adds no native frame). Pointer type is I64
    // (the JIT targets — x86-64 / aarch64 — are all 64-bit; the module is not built on wasm32).
    let guard_ptr: Option<ClifValue> = if recur_guard {
        // #187: the guard pointer is the LAST param — after the `arity` f64s (register-f64 ABI), or
        // after `[numeric_ptr, handles_ptr, deopt_ptr]` (array-mode ABI, index 3). Array-mode
        // self-recursion (queens' `place`) recurses on the native stack just like `fib`, so it needs
        // the SAME SP-bail to stay a catchable RangeError instead of a SIGSEGV (#381).
        let gp_idx = if array_mask != 0 { 3 } else { arity };
        let gp = *params.get(gp_idx)?;
        let stack_limit = bcx.ins().load(types::I64, MemFlags::new(), gp, 0);
        let sp = bcx.ins().get_stack_pointer(types::I64);
        let below = bcx.ins().icmp(IntCC::UnsignedLessThan, sp, stack_limit);
        let bail = bcx.create_block();
        let body = bcx.create_block();
        bcx.ins().brif(below, bail, &[], body, &[]);
        bcx.switch_to_block(bail);
        bcx.seal_block(bail);
        let one = bcx.ins().iconst(types::I8, 1);
        bcx.ins().store(MemFlags::new(), one, gp, 8); // RecurGuard.tripped (offset 8)
        let nan = bcx.ins().f64const(f64::NAN);
        bcx.ins().return_(&[nan]);
        blocks.insert(0, body); // internal jumps to offset 0 → the body, not the SP check
        bcx.switch_to_block(body);
        Some(gp)
    } else {
        None
    };
    let body_start = *blocks.get(&0)?; // `entry` normally; `body_block` when guarded

    // 2. A Variable per slot, all defined at entry so every path defines them. A JV slot (#189) holds
    //    an arena HANDLE (a `u64`, 0 = null) so its Variable is `i64`, not `f64`.
    let vars: Vec<Variable> = (0..num_slots)
        .map(|i| {
            let ty = if jv.is_some_and(|j| j.slots.contains(&i)) {
                types::I64
            } else {
                types::F64
            };
            bcx.declare_var(ty)
        })
        .collect();
    // Array-mode state: per-array-param `(ptr,len)` loaded from the handles array + the deopt pad.
    let mut array_slots: HashMap<usize, (ClifValue, ClifValue)> = HashMap::new();
    let mut deopt_block: Option<Block> = None;
    let mut deopt_ptr: Option<ClifValue> = None;
    if array_mask != 0 {
        // ABI params: [numeric_ptr, handles_ptr, deopt_ptr]. Numeric params load from numeric_ptr in
        // numeric-param order; array params load (ptr,len) from handles_ptr in array-param order.
        let numeric_ptr = *params.first()?;
        let handles_ptr = *params.get(1)?;
        deopt_ptr = Some(*params.get(2)?);
        let mut numeric_i = 0i32;
        let mut array_i = 0i64;
        #[allow(clippy::needless_range_loop)]
        // `slot` drives bit-mask math (`array_mask >> slot`) + map keys, not just indexing
        for slot in 0..num_slots {
            let init = if slot < arity && (array_mask >> slot) & 1 == 1 {
                let base = bcx.ins().iadd_imm(handles_ptr, array_i * 16);
                let p = bcx.ins().load(types::I64, MemFlags::new(), base, 0);
                let l = bcx.ins().load(types::I64, MemFlags::new(), base, 8);
                array_slots.insert(slot, (p, l));
                array_i += 1;
                bcx.ins().f64const(0.0) // an array slot's f64 Variable is never read
            } else if slot < arity {
                let v = bcx
                    .ins()
                    .load(types::F64, MemFlags::new(), numeric_ptr, numeric_i * 8);
                numeric_i += 1;
                v
            } else {
                bcx.ins().f64const(0.0)
            };
            bcx.def_var(vars[slot], init);
        }
        deopt_block = Some(bcx.create_block());
    } else {
        for (i, &v) in vars.iter().enumerate() {
            if jv.is_some_and(|j| j.slots.contains(&i)) {
                // A JV slot starts null (handle 0); its `[]` def stores the real handle. Null-init
                // makes the free-on-return safe even if a return precedes the def (`tish_jv_free`
                // treats handle 0 as a no-op).
                let null = bcx.ins().iconst(types::I64, 0);
                bcx.def_var(v, null);
            } else {
                let init = if i < arity {
                    params[i]
                } else {
                    bcx.ins().f64const(0.0)
                };
                bcx.def_var(v, init);
            }
        }
    }

    // 3. Translate. The operand stack is empty at every block boundary (statement-level control flow).
    let mut stack: Vec<JV> = Vec::new();
    // #189: JV array handles "in flight" (pushed by `LoadLocal`/`NewArray` of a JV slot, consumed by
    // the very next `GetIndex`/`SetIndex`/`GetMember`), kept off the f64 `stack`; and a pending
    // `arr.push` awaiting its arg + `Call`. Both must be empty at every block boundary.
    let mut jv_pending: Vec<ClifValue> = Vec::new();
    let mut pending_push: Option<ClifValue> = None;
    // #187: a resolved callee awaiting its args + `Call` (set by `LoadVar name`, consumed by the very
    // next `Call`). Like the JV pending state, must be empty at every block boundary.
    let mut pending_callee: Option<(cranelift::codegen::ir::FuncRef, u8)> = None;
    // #187: array-param slots passed as arguments to a self-recursive array-mode call (queens' `place`),
    // in argument order — pushed by `LoadLocal(array param)`, consumed by the next `SelfCall`.
    let mut arr_call_pending: Vec<usize> = Vec::new();
    let mut cur = body_start;
    let mut terminated = false;
    let mut ip = 0usize;
    while ip < code.len() {
        if let Some(&blk) = blocks.get(&ip) {
            if blk != cur {
                if !terminated {
                    if !stack.is_empty() {
                        return None;
                    }
                    bcx.ins().jump(blk, &[]);
                }
                // A JV ref, a pending push, a pending callee, or a pending array self-call arg must
                // never span a statement boundary.
                if !jv_pending.is_empty()
                    || pending_push.is_some()
                    || pending_callee.is_some()
                    || !arr_call_pending.is_empty()
                {
                    return None;
                }
                bcx.switch_to_block(blk);
                cur = blk;
                terminated = false;
                stack.clear();
            }
        }
        if terminated {
            // Skip the unreachable tail; a JV function may have array opcodes here (size via the full
            // table).
            let op = Opcode::from_u8(code[ip])?;
            ip += match op_size(op) {
                Some(s) => s,
                None if jv.is_some() => op.instruction_size(code, ip)?,
                None => return None,
            };
            continue;
        }
        let op = Opcode::from_u8(code[ip])?;
        match op {
            Opcode::LoadLocal => {
                let slot = peek_u16(code, ip + 1)? as usize;
                // #189: a JV slot's value is its arena HANDLE — push it to `jv_pending` (the next
                // GetIndex/SetIndex/GetMember consumes it), never onto the f64 stack.
                if jv.is_some_and(|j| j.slots.contains(&slot)) {
                    jv_pending.push(bcx.use_var(*vars.get(slot)?));
                    ip += 3;
                    continue;
                }
                // Array-mode peepholes on an array param, both bounds-checked (OOB → the deopt pad):
                //   READ  `LoadLocal(arr) ; (LoadLocal|LoadConst) idx ; GetIndex`         → native load
                //   WRITE `LoadLocal(arr) ; idx ; (LoadLocal|LoadConst) val ; Dup ; SetIndex` (#187) → store
                //   ARG   a bare `LoadLocal(arr)` → an argument to a self-recursive array call (#187)
                if array_mask != 0 && slot < arity && (array_mask >> slot) & 1 == 1 {
                    let (aptr, alen) = *array_slots.get(&slot)?;
                    // Decide the shape by PEEKING (no cranelift values created yet).
                    let at3 = code.get(ip + 3).copied().and_then(Opcode::from_u8);
                    let idx_simple =
                        matches!(at3, Some(Opcode::LoadLocal) | Some(Opcode::LoadConst));
                    let at6 = code.get(ip + 6).copied().and_then(Opcode::from_u8);
                    let is_read = idx_simple && at6 == Some(Opcode::GetIndex);
                    let is_write = idx_simple
                        && matches!(at6, Some(Opcode::LoadLocal) | Some(Opcode::LoadConst))
                        && code.get(ip + 9).copied().and_then(Opcode::from_u8) == Some(Opcode::Dup)
                        && code.get(ip + 10).copied().and_then(Opcode::from_u8)
                            == Some(Opcode::SetIndex);
                    if !is_read && !is_write {
                        // Bare array-param load — stage it as a self-call argument (the next `SelfCall`
                        // validates + consumes it). A genuine escape leaves it pending at a boundary → bail.
                        arr_call_pending.push(slot);
                        ip += 3;
                        continue;
                    }
                    let idx_f64 = read_simple_operand(&mut bcx, code, ip + 3, chunk, &vars)?;
                    // i = idx as usize (saturating: NaN→0, neg→0 — matches the VM's `n as usize`).
                    let i = bcx.ins().fcvt_to_uint_sat(types::I64, idx_f64);
                    // Read `val` (write only) BEFORE the bounds-check split so its LoadLocal reads the
                    // slot's current SSA value in `cur`, not the fresh `cont` block.
                    let store_val = if is_write {
                        Some(read_simple_operand(&mut bcx, code, ip + 6, chunk, &vars)?)
                    } else {
                        None
                    };
                    let inb = bcx.ins().icmp(IntCC::UnsignedLessThan, i, alen);
                    let cont = bcx.create_block();
                    let db = deopt_block?;
                    bcx.ins().brif(inb, cont, &[], db, &[]);
                    bcx.switch_to_block(cont);
                    cur = cont; // keep block-boundary tracking accurate after the mid-stream split
                    let off = bcx.ins().imul_imm(i, 8);
                    let addr = bcx.ins().iadd(aptr, off);
                    if let Some(val) = store_val {
                        bcx.ins().store(MemFlags::new(), val, addr, 0);
                        stack.push(JV::f64(val)); // assignment yields the value (a Pop usually discards it)
                        ip += 11; // LoadLocal(arr) + idx + val + Dup + SetIndex
                    } else {
                        let val = bcx.ins().load(types::F64, MemFlags::new(), addr, 0);
                        stack.push(JV::f64(val));
                        ip += 7; // LoadLocal(arr) + idx + GetIndex
                    }
                    continue;
                }
                let v = *vars.get(slot)?;
                // #187: a bool slot's `f64` 0/1 value carries the Bool repr so downstream
                // equality/return guards fire; a plain numeric slot pushes F64.
                let lv = bcx.use_var(v);
                stack.push(if bool_slots.contains(&slot) {
                    JV::boolean(lv)
                } else {
                    JV::f64(lv)
                });
                ip += 3;
            }
            Opcode::StoreLocal => {
                let slot = peek_u16(code, ip + 1)? as usize;
                // #189: storing a JV array (its `[]` def or a re-store) pops the handle from
                // `jv_pending` into the slot's i64 Variable.
                if jv.is_some_and(|j| j.slots.contains(&slot)) {
                    let ptr = jv_pending.pop()?;
                    bcx.def_var(*vars.get(slot)?, ptr);
                    ip += 3;
                    continue;
                }
                let jval = stack.pop()?;
                // #187: a boolean value may be stored only into a slot the pre-pass tagged as a bool
                // slot (represented as `f64` 0/1). Any other bool store → bail (keeps unknown shapes
                // on the interpreter). A numeric store into a bool-tagged slot is fine — the slot is
                // still an `f64`; its `LoadLocal`s just carry the Bool repr (conservatively).
                if jval.is_bool() && !bool_slots.contains(&slot) {
                    return None;
                }
                // Slots are f64 Variables — materialize an int repr once, at the store (an
                // `h = ((h<<13)|(h>>>19))>>>0` chain pays exactly ONE convert here, not per op).
                let val = jv_f64(&mut bcx, jval);
                let v = *vars.get(slot)?;
                bcx.def_var(v, val);
                ip += 3;
            }
            Opcode::Pop => {
                stack.pop()?;
                ip += 1;
            }
            Opcode::Dup => {
                let top = *stack.last()?;
                stack.push(top);
                ip += 1;
            }
            // Scope markers (EnterBlock/ExitBlock) + loop-var registration only affect the VM's
            // name-based block scope / per-iteration overlay — irrelevant to flat frame slots.
            Opcode::Nop | Opcode::EnterBlock | Opcode::ExitBlock | Opcode::LoopVarsEnd => ip += 1,
            Opcode::LoopVarsBegin => ip += 3,
            Opcode::Return => {
                let ret = stack.pop()?;
                if ret.is_bool() {
                    return None;
                }
                let v = jv_f64(&mut bcx, ret);
                // #189: free every JV array (return its arena slot to the free list) before leaving
                // the frame — handle 0 (a slot not yet allocated on this path) is a no-op. This is why
                // JV arrays must never escape.
                if let Some(jvc) = jv {
                    for &slot in jvc.slots {
                        let handle = bcx.use_var(*vars.get(slot)?);
                        bcx.ins().call(jvc.free, &[handle]);
                    }
                }
                bcx.ins().return_(&[v]);
                terminated = true;
                ip += 1;
            }
            Opcode::Jump => {
                let off = peek_u16(code, ip + 1)? as i16 as isize;
                let blk = *blocks.get(&(((ip + 3) as isize + off).max(0) as usize))?;
                if !stack.is_empty() {
                    return None;
                }
                bcx.ins().jump(blk, &[]);
                terminated = true;
                ip += 3;
            }
            Opcode::JumpBack => {
                let dist = peek_u16(code, ip + 1)? as usize;
                let blk = *blocks.get(&((ip + 3).checked_sub(dist)?))?;
                if !stack.is_empty() {
                    return None;
                }
                bcx.ins().jump(blk, &[]);
                terminated = true;
                ip += 3;
            }
            Opcode::JumpIfFalse => {
                let off = peek_u16(code, ip + 1)? as i16 as isize;
                let cond = stack.pop()?;
                if !stack.is_empty() {
                    return None; // non-empty stack ⇒ ternary shape ⇒ leave to build_body / VM
                }
                let cond = jv_f64(&mut bcx, cond);
                let falsy = falsy_flag(&mut bcx, cond);
                let target = *blocks.get(&(((ip + 3) as isize + off).max(0) as usize))?;
                let fallthrough = *blocks.get(&(ip + 3))?;
                bcx.ins().brif(falsy, target, &[], fallthrough, &[]);
                terminated = true;
                ip += 3;
            }
            Opcode::SelfCall if array_mask != 0 => {
                // #187: array-mode recursive self-call (queens' `place` recursing on the SAME arrays).
                // Array args were staged in `arr_call_pending` and MUST be exactly this function's own
                // array params in ABI order — then the recursion reuses `handles_ptr`/`deopt_ptr`
                // unchanged and only re-marshals the numeric args (a fresh `numeric_ptr` per level).
                let sref = self_ref?;
                let num_arrays = arr_call_pending.len();
                let num_numeric = arity.checked_sub(num_arrays)?;
                let expected: Vec<usize> =
                    (0..arity).filter(|k| (array_mask >> k) & 1 == 1).collect();
                if arr_call_pending != expected || stack.len() < num_numeric {
                    return None;
                }
                arr_call_pending.clear();
                // Marshal the numeric args (top of the stack, in ABI order) into a fresh stack slot.
                let slot = bcx.create_sized_stack_slot(cranelift::codegen::ir::StackSlotData::new(
                    cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                    (num_numeric * 8) as u32,
                    3, // 2^3 = 8-byte alignment for f64
                ));
                let arg_start = stack.len() - num_numeric;
                let args: Vec<JV> = stack.drain(arg_start..).collect();
                for (j, jv) in args.into_iter().enumerate() {
                    if jv.is_bool() {
                        return None; // a bool numeric arg doesn't match the f64 ABI
                    }
                    let v = jv_f64(&mut bcx, jv);
                    bcx.ins().stack_store(v, slot, (j * 8) as i32);
                }
                let num_ptr = bcx.ins().stack_addr(types::I64, slot, 0);
                let handles_ptr = *params.get(1)?;
                let deopt_ptr = *params.get(2)?;
                // #187: thread the RecurGuard (if this recursive array fn was compiled with one) so every
                // level re-checks the native stack at entry — a catchable RangeError, not a SIGSEGV.
                let mut call_args = vec![num_ptr, handles_ptr, deopt_ptr];
                if let Some(gp) = guard_ptr {
                    call_args.push(gp);
                }
                let call = bcx.ins().call(sref, &call_args);
                let res = bcx.inst_results(call)[0];
                stack.push(JV::f64(res));
                ip += 3;
            }
            Opcode::SelfCall => {
                // Recursive self-call → a native cranelift call to this very function. Args are the
                // top `arity` f64 stack values; result is pushed. Validated above (self_ref present,
                // arity matches). This is what makes `fib` etc. run at native speed.
                let sref = self_ref?; // guaranteed Some by the validation scan
                if stack.len() < arity {
                    return None;
                }
                let arg_start = stack.len() - arity;
                let args: Vec<JV> = stack.drain(arg_start..).collect();
                let mut call_args = Vec::with_capacity(arity);
                for jv in args {
                    if jv.is_bool() {
                        return None; // boolean args don't match the f64 ABI
                    }
                    call_args.push(jv_f64(&mut bcx, jv));
                }
                // #381: thread the RecurGuard pointer through the recursive call so every level
                // re-checks the stack at its entry. Present iff this function was compiled guarded.
                if let Some(gp) = guard_ptr {
                    call_args.push(gp);
                }
                let call = bcx.ins().call(sref, &call_args);
                let result = bcx.inst_results(call)[0];
                stack.push(JV::f64(result));
                ip += 3;
            }
            Opcode::MathUnary => {
                // #186 — `Math.<fn>(x)`: pop the arg, emit native op / host call, push the result.
                let id = peek_u16(code, ip + 1)?;
                let mfn = MathUnaryFn::from_u16(id)?;
                let x = stack.pop()?;
                let x = jv_f64(&mut bcx, x);
                let r = emit_math_unary(&mut bcx, math_fref, mfn, x);
                stack.push(JV::f64(r));
                ip += 3;
            }
            // #189 — local-array ops, only for JV functions (`jv` = `Some`). Each consumes the array
            // HANDLE from `jv_pending` (its `LoadLocal`/`NewArray` pushed it) and calls `tish_jv_*`.
            Opcode::NewArray if jv.is_some() => {
                let jvc = jv?;
                if peek_u16(code, ip + 1)? != 0 {
                    return None; // only empty `[]` is JV (classifier already guaranteed this)
                }
                let cap = bcx.ins().iconst(types::I64, 0);
                let call = bcx.ins().call(jvc.new, &[cap]);
                jv_pending.push(bcx.inst_results(call)[0]);
                ip += 3;
            }
            Opcode::GetIndex if !jv_pending.is_empty() => {
                let jvc = jv?;
                let idx = stack.pop()?;
                let idx = jv_f64(&mut bcx, idx);
                let handle = jv_pending.pop()?;
                // `idx as usize` (saturating: NaN/neg → 0), matching the VM's index coercion. OOB sets
                // the per-thread deopt flag inside `tish_jv_get` and returns NaN.
                let i = bcx.ins().fcvt_to_uint_sat(types::I64, idx);
                let call = bcx.ins().call(jvc.get, &[handle, i]);
                let res = bcx.inst_results(call)[0];
                stack.push(JV::f64(res));
                ip += 1;
            }
            Opcode::SetIndex if !jv_pending.is_empty() => {
                let jvc = jv?;
                // Stack: [ (array→jv_pending), idx, val, dup_val ]. `Dup` left `dup_val` == `val`.
                let dup_jv = stack.pop()?;
                let _val = stack.pop()?;
                let idx = stack.pop()?;
                let dup_val = jv_f64(&mut bcx, dup_jv);
                let idx = jv_f64(&mut bcx, idx);
                let handle = jv_pending.pop()?;
                let i = bcx.ins().fcvt_to_uint_sat(types::I64, idx);
                bcx.ins().call(jvc.set, &[handle, i, dup_val]); // OOB → deopt flag inside tish_jv_set
                stack.push(JV::f64(dup_val)); // assignment yields the value
                ip += 1;
            }
            Opcode::GetMember if !jv_pending.is_empty() => {
                let jvc = jv?;
                let name = chunk.names.get(peek_u16(code, ip + 1)? as usize)?;
                let ptr = jv_pending.pop()?;
                match name.as_ref() {
                    "length" => {
                        let call = bcx.ins().call(jvc.len, &[ptr]);
                        let len_i = bcx.inst_results(call)[0];
                        let len_f = bcx.ins().fcvt_from_uint(types::F64, len_i);
                        stack.push(JV::f64(len_f));
                    }
                    "push" => pending_push = Some(ptr),
                    _ => return None, // any other member of a JV array → bail
                }
                ip += 3;
            }
            Opcode::Call if pending_push.is_some() => {
                let jvc = jv?;
                if peek_u16(code, ip + 1)? != 1 {
                    return None; // `push` takes exactly one arg in the JV fast path
                }
                let ptr = pending_push.take()?;
                let arg = stack.pop()?;
                let arg = jv_f64(&mut bcx, arg);
                bcx.ins().call(jvc.push, &[ptr, arg]);
                // `Array.push` returns the new length.
                let call = bcx.ins().call(jvc.len, &[ptr]);
                let len_i = bcx.inst_results(call)[0];
                let len_f = bcx.ins().fcvt_from_uint(types::F64, len_i);
                stack.push(JV::f64(len_f));
                ip += 3;
            }
            // #187: `LoadVar name` where `name` is a resolved directly-callable callee — stage it for the
            // very next `Call`. Any other `LoadVar` (an unresolved global) bails to the interpreter.
            Opcode::LoadVar => {
                if pending_callee.is_some() {
                    return None; // no nested resolved calls in v1
                }
                let name = chunk.names.get(peek_u16(code, ip + 1)? as usize)?;
                match resolved.get(name) {
                    Some(&(fref, callee_arity)) => pending_callee = Some((fref, callee_arity)),
                    None => return None,
                }
                ip += 3;
            }
            // #187: `Call argc` consuming a staged callee — a native cranelift call (like `SelfCall`,
            // minus the guard). Pops `argc` f64 args from the stack, pushes the f64 result.
            Opcode::Call if pending_callee.is_some() => {
                let (fref, callee_arity) = pending_callee.take()?;
                if peek_u16(code, ip + 1)? as u8 != callee_arity
                    || stack.len() < callee_arity as usize
                {
                    return None;
                }
                let arg_start = stack.len() - callee_arity as usize;
                let args: Vec<JV> = stack.drain(arg_start..).collect();
                let mut call_args = Vec::with_capacity(callee_arity as usize);
                for jv in args {
                    if jv.is_bool() {
                        return None; // a bool arg doesn't match the callee's f64 ABI
                    }
                    call_args.push(jv_f64(&mut bcx, jv));
                }
                let call = bcx.ins().call(fref, &call_args);
                let res = bcx.inst_results(call)[0];
                stack.push(JV::f64(res));
                ip += 3;
            }
            // #187: a VOID array-mode function's implicit `return null` (the fall-through of a
            // side-effect writer like `multiplyAv`). Emit a dummy `f64` result — the wrapper returns
            // `Value::Null` for a void function, matching the interpreter — so the whole native body
            // (the array reads/writes + any cross-calls) runs instead of bailing on the null. MUST be
            // an actual return-null (next op is `Return`): a MID-BODY `let x = null` is also a
            // non-numeric `LoadConst`, and terminating on it would drop the rest of the function.
            Opcode::LoadConst
                if is_void
                    && Opcode::from_u8(*code.get(ip + 3)?)? == Opcode::Return
                    && !matches!(
                        chunk.constants.get(peek_u16(code, ip + 1)? as usize),
                        Some(Constant::Number(_)) | Some(Constant::Bool(_))
                    ) =>
            {
                let zero = bcx.ins().f64const(0.0);
                bcx.ins().return_(&[zero]);
                terminated = true; // the following `Return` (and any dead tail) is now skipped
                ip += 3;
            }
            // #189: a non-numeric constant load in a JV function — almost always the compiler's
            // trailing implicit `LoadConst Null; Return` epilogue after a `while (true)` loop. That
            // epilogue is the loop's never-taken exit edge: statically reachable (so it becomes a
            // cranelift block that must be filled), dynamically dead. A `null`/string can't live in
            // an f64 register, so instead of bailing the whole function we emit a deopt here — free
            // the live arrays, set the flag, and return. If the block truly never runs (infinite loop
            // with an inner `return`), cranelift keeps a dead branch and the hot path stays native;
            // if a non-numeric return is ever actually reached, the VM wrapper re-interprets and
            // produces the correct value. Sound either way, since the deopt path reproduces semantics.
            Opcode::LoadConst
                if jv.is_some()
                    && Opcode::from_u8(*code.get(ip + 3)?)? == Opcode::Return
                    && !matches!(
                        chunk.constants.get(peek_u16(code, ip + 1)? as usize),
                        Some(Constant::Number(_)) | Some(Constant::Bool(_))
                    ) =>
            {
                let jvc = jv?;
                for &slot in jvc.slots {
                    let handle = bcx.use_var(*vars.get(slot)?);
                    bcx.ins().call(jvc.free, &[handle]);
                }
                bcx.ins().call(jvc.deopt, &[]);
                let zero = bcx.ins().f64const(0.0);
                bcx.ins().return_(&[zero]);
                terminated = true; // the following `Return` (and any dead tail) is now skipped
                ip += 3;
            }
            _ => match emit_simple_op(&mut bcx, chunk, code, &mut ip, &mut stack, &params, arity) {
                SimpleOp::Handled(_) => {}
                _ => return None, // LoadConst/BinOp/UnaryOp handled; anything else → VM
            },
        }
    }

    if !terminated {
        return None;
    }
    // Array-mode deopt landing pad: `*deopt = 1; return 0.0`. Reached from every OOB bounds-check; the
    // VM wrapper sees the flag and re-runs the interpreter (so OOB → `Value::Null` stays correct).
    if let (Some(db), Some(dp)) = (deopt_block, deopt_ptr) {
        bcx.switch_to_block(db);
        let one = bcx.ins().iconst(types::I8, 1);
        bcx.ins().store(MemFlags::new(), one, dp, 0);
        let zero = bcx.ins().f64const(0.0);
        bcx.ins().return_(&[zero]);
    }
    bcx.seal_all_blocks();
    bcx.finalize();
    Some(false)
}

fn build_body(
    func: &mut cranelift::codegen::ir::Function,
    fbctx: &mut FunctionBuilderContext,
    chunk: &Chunk,
    arity: usize,
) -> Option<bool> {
    let mut bcx = FunctionBuilder::new(func, fbctx);
    let entry = bcx.create_block();
    bcx.append_block_params_for_function_params(entry);
    bcx.switch_to_block(entry);
    bcx.seal_block(entry);
    let params: Vec<ClifValue> = bcx.block_params(entry).to_vec();

    let code = &chunk.code;
    // Each entry is a typed [`JV`]. The Bool repr marks comparison/`!` results
    // (logical 0.0/1.0) so the final value boxes as Bool, not Number; integer
    // reprs materialize to f64 at the Return / select boundaries below.
    let mut stack: Vec<JV> = Vec::new();
    let mut ip = 0usize;
    let mut result: Option<bool> = None;

    while ip < code.len() {
        match emit_simple_op(&mut bcx, chunk, code, &mut ip, &mut stack, &params, arity) {
            SimpleOp::Handled(_) => continue,
            SimpleOp::Unsupported => return None,
            SimpleOp::NotSimple => {}
        }
        let op = Opcode::from_u8(code[ip])?;
        match op {
            Opcode::Return => {
                let jv = stack.pop()?;
                let v = jv_f64(&mut bcx, jv);
                bcx.ins().return_(&[v]);
                result = Some(jv.is_bool()); // first Return ends a (sub)path
                break;
            }
            // Ternary `cond ? A : B` → `select`. Both arms must be branch-free numeric
            // sub-sequences, each pushing exactly one value, with matching is_bool.
            Opcode::JumpIfFalse => {
                let cond = stack.pop()?;
                let cond = jv_f64(&mut bcx, cond);
                let mut p = ip + 1;
                let off = read_u16(code, &mut p)? as i16 as isize; // p now past the operand
                let else_target = (p as isize + off).max(0) as usize;
                let base = stack.len();

                // THEN arm: straight-line ops until the trailing `Jump`.
                let mut tip = p;
                loop {
                    match emit_simple_op(
                        &mut bcx, chunk, code, &mut tip, &mut stack, &params, arity,
                    ) {
                        SimpleOp::Handled(_) => continue,
                        SimpleOp::Unsupported => return None,
                        SimpleOp::NotSimple => break,
                    }
                }
                if Opcode::from_u8(*code.get(tip)?)? != Opcode::Jump {
                    return None; // not the ternary shape (e.g. early return) → VM
                }
                let mut jp = tip + 1;
                let joff = read_u16(code, &mut jp)? as i16 as isize;
                let merge_target = (jp as isize + joff).max(0) as usize;
                // The else arm must begin exactly where the then-arm's `Jump` left off.
                if else_target != jp || stack.len() != base + 1 {
                    return None;
                }
                let then_jv = stack.pop()?;
                let then_v = jv_f64(&mut bcx, then_jv);

                // ELSE arm: straight-line ops from `jp` up to the merge point.
                let mut eip = jp;
                while eip < merge_target {
                    match emit_simple_op(
                        &mut bcx, chunk, code, &mut eip, &mut stack, &params, arity,
                    ) {
                        SimpleOp::Handled(_) => continue,
                        _ => return None, // nested control flow / unsupported → VM
                    }
                }
                if eip != merge_target || stack.len() != base + 1 {
                    return None;
                }
                let else_jv = stack.pop()?;
                let else_v = jv_f64(&mut bcx, else_jv);
                // One result_bool per function: arms must agree on Bool-vs-Number.
                if then_jv.is_bool() != else_jv.is_bool() {
                    return None;
                }

                let falsy = falsy_flag(&mut bcx, cond);
                let sel = bcx.ins().select(falsy, else_v, then_v);
                stack.push(if then_jv.is_bool() {
                    JV::boolean(sel)
                } else {
                    JV::f64(sel)
                });
                ip = merge_target;
            }
            // #187: a `function name(x) { … }` block body wraps its statements in EnterBlock/ExitBlock
            // (+ loop-var markers) — pure scope bookkeeping with no runtime effect on a straight-line
            // numeric body. Skip them so leaf functions (e.g. a `sq(x)`/`evalA(i,j)` cross-call callee)
            // compile instead of bailing on the first marker. The real `Return` still ends the path.
            Opcode::EnterBlock | Opcode::ExitBlock | Opcode::LoopVarsEnd => ip += 1,
            Opcode::LoopVarsBegin => ip += 3,
            // Loops / member / index / call / array / object → VM.
            _ => return None,
        }
    }

    bcx.finalize();
    result
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use tishlang_core::{to_int32, to_uint32};

    /// First arity-2 slot-based numeric nested chunk in compiled `src` (cloned, so the caller owns it).
    fn fn_chunk(src: &str) -> Chunk {
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let top = tishlang_bytecode::compile(&opt).expect("compile");
        fn find(c: &Chunk) -> Option<Chunk> {
            for n in &c.nested {
                if n.slot_based && n.param_count == 2 && n.rest_param_index == NO_REST_PARAM {
                    return Some(n.clone());
                }
                if let Some(x) = find(n) {
                    return Some(x);
                }
            }
            None
        }
        find(&top).expect("expected an arity-2 slot-based fn chunk")
    }

    /// Compile `src`, then return the first arity-2 pure-numeric function the JIT accepts. Panics if
    /// none compiles — so a change that makes the JIT silently *stop* compiling the target (the exact
    /// "vacuous fixture" miss that motivated this guard) fails loudly instead of passing emptily.
    ///
    /// Bypasses [`try_compile_numeric`]'s cache and calls [`compile_chunk`] directly: the cache is
    /// keyed by chunk address, unique-and-stable in a real run but reused across this test's transient
    /// chunks. Compiling fresh is correct here and still exercises the real lowering path.
    fn jit_arity2(src: &str) -> NumericFn {
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let chunk = tishlang_bytecode::compile(&opt).expect("compile");
        fn compile_uncached(c: &Chunk) -> Option<NumericFn> {
            if !c.slot_based
                || c.rest_param_index != NO_REST_PARAM
                || c.param_count == 0
                || c.param_count > 8
            {
                return None;
            }
            let lock = jit()?;
            let mut g = lock.lock().ok()?;
            compile_chunk(&mut g, c)
        }
        fn find(c: &Chunk) -> Option<NumericFn> {
            for n in &c.nested {
                if let Some(f) = compile_uncached(n) {
                    if f.arity == 2 && f.array_param_mask == 0 {
                        return Some(f);
                    }
                }
                if let Some(f) = find(n) {
                    return Some(f);
                }
            }
            None
        }
        find(&chunk).expect("the JIT must compile this arity-2 numeric fn (did it start bailing?)")
    }

    /// #189: compile the first JV (function-local `f64` array) nested fn in `src` via `compile_chunk`,
    /// bypassing the address-keyed cache (see [`jit_arity2`]). Panics if none compiles JV — so a change
    /// that makes the classifier or lowering silently stop accepting the target fails loudly.
    fn jit_jv(src: &str) -> NumericFn {
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let chunk = tishlang_bytecode::compile(&opt).expect("compile");
        fn compile_uncached(c: &Chunk) -> Option<NumericFn> {
            if !c.slot_based
                || c.rest_param_index != NO_REST_PARAM
                || c.param_count == 0
                || c.param_count > 8
            {
                return None;
            }
            let lock = jit()?;
            let mut g = lock.lock().ok()?;
            compile_chunk(&mut g, c)
        }
        fn find(c: &Chunk) -> Option<NumericFn> {
            for n in &c.nested {
                if let Some(f) = compile_uncached(n) {
                    if f.is_jv() {
                        return Some(f);
                    }
                }
                if let Some(f) = find(n) {
                    return Some(f);
                }
            }
            None
        }
        find(&chunk).expect("the JIT must JV-compile this local-array fn (did it start bailing?)")
    }

    /// #189: the core JV path — a function-local array built with `push`, then read and written by
    /// index in a loop. `sum_{i=0}^{n-1}(2i+1) == n^2`, so the JIT'd result is checked against a
    /// closed form (independent of the interpreter) across sizes. `deopt` must stay clear (in-bounds).
    #[test]
    fn jit_jv_local_array_sum_matches_closed_form() {
        let f = jit_jv(
            "function build(n) {\n\
             let a = []\n\
             for (let i = 0; i < n; i = i + 1) { a.push(i * 2) }\n\
             let s = 0\n\
             let j = 0\n\
             while (j < n) { a[j] = a[j] + 1; s = s + a[j]; j = j + 1 }\n\
             return s\n\
             }\n\
             build(0)\n",
        );
        assert!(f.is_jv(), "expected a JV-compiled fn");
        for n in [1.0f64, 5.0, 20.0, 100.0] {
            jv_reset_deopt();
            let r = f.call(&[n]);
            assert!(!jv_take_deopt(), "no OOB expected for n={n}");
            assert_eq!(r, n * n, "JV local-array sum wrong for n={n}");
        }
    }

    /// #189: a `while (true)` whose only exit is an inner `return` — the compiler still emits a trailing
    /// implicit `LoadConst Null; Return` epilogue (the loop's never-taken exit edge). That epilogue must
    /// NOT bail JV compilation; it's routed to a deopt. Without the fix this fn falls back to the VM.
    #[test]
    fn jit_jv_while_true_epilogue_compiles() {
        let f = jit_jv(
            "function f(n) {\n\
             let a = []\n\
             a.push(0)\n\
             let i = 0\n\
             while (true) {\n\
               a[0] = a[0] + 1\n\
               i = i + 1\n\
               if (i >= n) { return a[0] }\n\
             }\n\
             }\n\
             f(0)\n",
        );
        assert!(f.is_jv(), "while(true)+return JV fn must compile, not bail");
        jv_reset_deopt();
        assert_eq!(f.call(&[7.0]), 7.0);
        assert!(!jv_take_deopt());
    }

    /// #189: an out-of-bounds index makes the host `tish_jv_get` set the deopt flag (and return a
    /// sentinel); the VM wrapper then discards the result and re-interprets. Sound because JV arrays
    /// never escape, so the partially-run native attempt mutated nothing observable.
    #[test]
    fn jit_jv_oob_read_sets_deopt_flag() {
        // A loop that reads `a[i]` for `i` in `0..n`, but the array only has 2 elements — so `n > 2`
        // reads out of bounds. (Needs a loop: the CFG JIT only engages on loop/self-call fns.)
        let f = jit_jv(
            "function f(n) {\n\
             let a = []\n\
             a.push(10)\n\
             a.push(20)\n\
             let s = 0\n\
             for (let i = 0; i < n; i = i + 1) { s = s + a[i] }\n\
             return s\n\
             }\n\
             f(0)\n",
        );
        jv_reset_deopt();
        assert_eq!(f.call(&[2.0]), 30.0, "a[0]+a[1] == 30");
        assert!(!jv_take_deopt(), "n=2 stays in bounds");
        jv_reset_deopt();
        let _ = f.call(&[5.0]); // reads a[2] (past the 2-element array) → OOB
        assert!(jv_take_deopt(), "OOB array access must set the deopt flag");
    }

    /// #187: compile the first arity-1 plain-numeric (non-JV, non-array) fn in `src` via `compile_chunk`.
    fn jit_numeric1(src: &str) -> NumericFn {
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let chunk = tishlang_bytecode::compile(&opt).expect("compile");
        fn compile_uncached(c: &Chunk) -> Option<NumericFn> {
            if !c.slot_based || c.rest_param_index != NO_REST_PARAM || c.param_count != 1 {
                return None;
            }
            let lock = jit()?;
            let mut g = lock.lock().ok()?;
            compile_chunk(&mut g, c)
        }
        fn find(c: &Chunk) -> Option<NumericFn> {
            for n in &c.nested {
                if let Some(f) = compile_uncached(n) {
                    if !f.is_jv() && f.array_param_mask() == 0 {
                        return Some(f);
                    }
                }
                if let Some(f) = find(n) {
                    return Some(f);
                }
            }
            None
        }
        find(&chunk).expect("the JIT must compile this arity-1 numeric fn (did it start bailing?)")
    }

    /// #187: a boolean SCALAR local (`let done = false; … done = true; if (done) …`) must JIT — the
    /// bool is represented as an `f64` 0/1 and drives `if (done)` via the falsy check. Result is the
    /// numeric accumulator, checked against a closed form (`sum_{i<n} i == n(n-1)/2`), independent of
    /// the interpreter. Without bool slots this fn bails at the `done = false` StoreLocal → interprets.
    #[test]
    fn jit_bool_scalar_slot_flips_and_matches() {
        // Bounded loop (not `while(true)`) so this exercises the bool slot in isolation — a bool slot
        // `done` set inside a branch, then driving `if (done)`. `sum_{i<n} i == n(n-1)/2` for all n≥0.
        let f = jit_numeric1(
            "function f(n) {\n\
             let done = false\n\
             let acc = 0\n\
             for (let i = 0; i < n; i = i + 1) {\n\
               acc = acc + i\n\
               if (i + 1 >= n) { done = true }\n\
             }\n\
             if (done) { return acc }\n\
             return 0\n\
             }\n\
             f(0)\n",
        );
        for n in [0i64, 1, 2, 10, 100, 1000] {
            let expect = (n * (n - 1) / 2) as f64;
            assert_eq!(
                f.call(&[n as f64]),
                expect,
                "bool-slot loop wrong for n={n}"
            );
        }
    }

    /// #187 soundness: a `bool === number` compare must NOT be JIT'd to an `f64` compare (JS says
    /// `0 === false` / `true === 1` is FALSE across types, but the bits `1.0 === 1.0` are equal). The
    /// equality guard in `emit_simple_op` bails such a function to the interpreter. This uses a LOOP so
    /// it reaches the CFG JIT (+ the guard); WITHOUT the guard it would compile and wrongly count every
    /// iteration (returning n), so the closed-form assertion catches a regression. WITH the guard it
    /// bails (no `NumericFn`), and the assertion is vacuously satisfied — either way it never miscompiles.
    #[test]
    fn jit_bool_eq_number_never_miscompiles() {
        let src = "function g(n) {\n\
                   let flag = false\n\
                   let hits = 0\n\
                   for (let i = 0; i < n; i = i + 1) {\n\
                     flag = i >= 0\n\
                     if (flag === 1) { hits = hits + 1 }\n\
                   }\n\
                   return hits\n\
                   }\n\
                   g(0)\n";
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let chunk = tishlang_bytecode::compile(&opt).expect("compile");
        fn first_num1(c: &Chunk) -> Option<NumericFn> {
            for n in &c.nested {
                if n.slot_based && n.param_count == 1 {
                    if let Some(g) =
                        jit().and_then(|l| l.lock().ok().and_then(|mut g| compile_chunk(&mut g, n)))
                    {
                        return Some(g);
                    }
                }
                if let Some(g) = first_num1(n) {
                    return Some(g);
                }
            }
            None
        }
        if let Some(g) = first_num1(&chunk) {
            // `(i>=0) === 1` is always FALSE in JS, so `hits` must stay 0 — never `n`.
            assert_eq!(
                g.call(&[5.0]),
                0.0,
                "`bool === 1` must be false → hits stays 0"
            );
        }
    }

    /// #187: compile the first array-mode fn (`array_param_mask != 0`) in `src` via `compile_chunk`.
    fn jit_arrays(src: &str) -> NumericFn {
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let chunk = tishlang_bytecode::compile(&opt).expect("compile");
        fn compile_uncached(c: &Chunk) -> Option<NumericFn> {
            if !c.slot_based || c.rest_param_index != NO_REST_PARAM || c.param_count == 0 {
                return None;
            }
            let lock = jit()?;
            let mut g = lock.lock().ok()?;
            compile_chunk(&mut g, c)
        }
        fn find(c: &Chunk) -> Option<NumericFn> {
            for n in &c.nested {
                if let Some(f) = compile_uncached(n) {
                    if f.array_param_mask() != 0 {
                        return Some(f);
                    }
                }
                if let Some(f) = find(n) {
                    return Some(f);
                }
            }
            None
        }
        find(&chunk)
            .expect("the JIT must array-compile this fn (did classify_params stop matching?)")
    }

    /// #187: an array param written as `dst[i] = v` is stored into and copied back. Here `copy` reads
    /// `src[i]` into `dst[i]`; after `call_arrays` the caller's `dst` buffer must equal `src`, and the
    /// writable mask must flag `dst` (so [`try_call_array_jit`] knows to write it back).
    #[test]
    fn jit_array_param_write_and_readback() {
        let f = jit_arrays(
            "function copy(n, src, dst) {\n\
             let i = 0\n\
             while (i < n) { let x = src[i]; dst[i] = x; i = i + 1 }\n\
             return dst[0]\n\
             }\n\
             copy(0, [], [])\n",
        );
        assert!(f.array_param_mask() != 0, "src/dst are array params");
        assert_ne!(f.array_writable_mask(), 0, "dst must be flagged writable");
        let mut src = [10.0f64, 20.0, 30.0];
        let mut dst = [0.0f64; 3];
        // arity 3 = [n (numeric), src (array), dst (array)]. numeric = [n]; arrays in param order.
        let handles = [
            ArrayHandle {
                ptr: src.as_mut_ptr(),
                len: 3,
            },
            ArrayHandle {
                ptr: dst.as_mut_ptr(),
                len: 3,
            },
        ];
        let (res, deopt) = f.call_arrays(&[3.0], &handles);
        assert!(!deopt, "in-bounds writes never deopt");
        assert_eq!(dst, [10.0, 20.0, 30.0], "dst must be overwritten with src");
        assert_eq!(res, 10.0, "returns dst[0]");
    }

    /// #187: an out-of-bounds array-param WRITE sets the deopt flag (the wrapper then discards the
    /// scratch — no partial writeback — and re-interprets).
    #[test]
    fn jit_array_param_write_oob_deopts() {
        let f = jit_arrays(
            "function copy(n, src, dst) {\n\
             let i = 0\n\
             while (i < n) { let x = src[i]; dst[i] = x; i = i + 1 }\n\
             return dst[0]\n\
             }\n\
             copy(0, [], [])\n",
        );
        let mut src = [1.0f64; 5];
        let mut dst = [0.0f64; 2]; // only 2 elements — a write to dst[2] is OOB
        let handles = [
            ArrayHandle {
                ptr: src.as_mut_ptr(),
                len: 5,
            },
            ArrayHandle {
                ptr: dst.as_mut_ptr(),
                len: 2,
            },
        ];
        let (_res, deopt) = f.call_arrays(&[5.0], &handles); // writes dst[0..5) into a len-2 array
        assert!(deopt, "OOB array-param write must deopt");
    }

    /// #187: a caller lowers a `name(args)` call to a stable register-`f64` callee into a native
    /// cranelift call. Compiles `sq` first (registering it), then `sumSq` which calls it; the result
    /// (`sum_{i<n} i^2 == (n-1)n(2n-1)/6`) is checked against a closed form independent of the interp.
    #[test]
    fn jit_cross_function_call_matches_closed_form() {
        let src = "function sq(x) { return x * x }\n\
                   function sumSq(n) {\n\
                     let s = 0\n\
                     let i = 0\n\
                     while (i < n) { s = s + sq(i); i = i + 1 }\n\
                     return s\n\
                   }\n\
                   sumSq(0)\n";
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        let chunk = tishlang_bytecode::compile(&opt).expect("compile");
        // Compile every nested fn in source order (sq before sumSq) so sq registers before sumSq
        // resolves it. `compile_chunk` bypasses the address cache, and cross-call fns aren't cached.
        let lock = jit().expect("jit available");
        let mut sumsq: Option<NumericFn> = None;
        for n in &chunk.nested {
            let mut g = lock.lock().unwrap();
            if let Some(f) = compile_chunk(&mut g, n) {
                if n.global_name.as_deref() == Some("sumSq") {
                    sumsq = Some(f);
                }
            }
        }
        let f =
            sumsq.expect("sumSq must compile with the resolved `sq` call (did resolution break?)");
        for n in [1i64, 5, 20, 50] {
            let expect = ((n - 1) * n * (2 * n - 1) / 6) as f64;
            assert_eq!(
                f.call(&[n as f64]),
                expect,
                "cross-call sumSq wrong for n={n}"
            );
        }
    }

    /// #187: an array-mode function that is SELF-RECURSIVE and passes a BOOL array param back to itself
    /// (queens' `place` shape). Exercises: bool-array element read (`!cols[col]`), bool-array write
    /// (`cols[col] = true/false`), and the native array self-call (reusing `handles_ptr`, re-marshalling
    /// the numeric args). `place(n, 0, all-false)` counts permutations = n!.
    #[test]
    fn jit_array_mode_recursion_over_bool_array() {
        let f = jit_arrays(
            "function place(n, row, cols) {\n\
             if (row === n) { return 1 }\n\
             let count = 0\n\
             for (let col = 0; col < n; col = col + 1) {\n\
               if (!cols[col]) {\n\
                 cols[col] = true\n\
                 count = count + place(n, row + 1, cols)\n\
                 cols[col] = false\n\
               }\n\
             }\n\
             return count\n\
             }\n\
             place(0, 0, [])\n",
        );
        assert!(f.array_param_mask() != 0, "cols is an array param");
        assert!(
            f.recur_guarded(),
            "place is self-recursive → compiled with a trailing RecurGuard param"
        );
        // n! for n = 0..6 (cols starts all-false = 0.0). Each call resets `cols` to all-false on return.
        let factorial = [1.0f64, 1.0, 2.0, 6.0, 24.0, 120.0, 720.0];
        for (n, &expect) in factorial.iter().enumerate() {
            let mut cols = vec![0.0f64; n.max(1)];
            let handles = [ArrayHandle {
                ptr: cols.as_mut_ptr(),
                len: n,
            }];
            // `stack_limit = 0` ⇒ the entry SP-check never trips (a real SP is always ≥ 0).
            let mut guard = RecurGuard {
                stack_limit: 0,
                tripped: 0,
            };
            let (res, deopt) = f.call_arrays_guarded(&[n as f64, 0.0], &handles, &mut guard);
            assert!(!deopt, "in-bounds, no deopt for n={n}");
            assert_eq!(
                guard.tripped, 0,
                "shallow recursion must not trip the guard"
            );
            assert_eq!(res, expect, "place({n},0,cols) = {n}! = {expect}");
        }
    }

    /// #187: the write-KIND classification that pins the array-mode writeback's soundness. The JIT
    /// flattens every element to `f64`, so `try_call_array_jit` re-boxes each written element by the
    /// runtime array's element type — which is only sound when the statically written kind (bool vs
    /// number) matches. `classify_params` must therefore report, per writable array, whether it is
    /// written with a bool (a `LoadConst(Bool)` or a `LoadLocal(bool-slot)`), and reject a single array
    /// written with BOTH a bool and a number (unrepresentable as one flat re-boxing).
    #[test]
    fn classify_params_bool_write_kinds() {
        // The arity-2 array fn `f(a, n)` from `src` (already the nested chunk).
        fn arr_fn(src: &str) -> Chunk {
            fn_chunk(src)
        }
        let bit0 = |m: u8| m & 1;

        // A bool CONST write ⇒ array bit, writable bit, AND bool-write bit all set.
        let c = arr_fn(
            "function f(a, n) { for (let i = 0; i < n; i = i + 1) { a[i] = true } return 0 }\nf([0], 1)\n",
        );
        let (am, wm, bm) = classify_params(&c, c.param_count as usize);
        assert_eq!(
            (bit0(am), bit0(wm), bit0(bm)),
            (1, 1, 1),
            "bool-const write"
        );

        // A number CONST write ⇒ array + writable set, bool-write CLEAR.
        let c = arr_fn(
            "function f(a, n) { for (let i = 0; i < n; i = i + 1) { a[i] = 7 } return 0 }\nf([0], 1)\n",
        );
        let (am, wm, bm) = classify_params(&c, c.param_count as usize);
        assert_eq!(
            (bit0(am), bit0(wm), bit0(bm)),
            (1, 1, 0),
            "number-const write"
        );

        // A bool LOCAL write ⇒ the whole fn bails: `classify_bool_slots` is a superset (a bool-tagged
        // slot may be reassigned a number), so a `LoadLocal(bool-slot)` written value can't be typed for
        // the flat writeback. Reject rather than risk re-boxing wrong.
        let c = arr_fn(
            "function f(a, n) { let b = true; for (let i = 0; i < n; i = i + 1) { a[i] = b } return 0 }\nf([0], 1)\n",
        );
        assert_eq!(
            classify_params(&c, c.param_count as usize),
            (0, 0, 0),
            "bool-local write must bail (bool-tagged slot may hold a number)"
        );

        // A number LOCAL write ⇒ bool-write CLEAR.
        let c = arr_fn(
            "function f(a, n) { let x = n + 1; for (let i = 0; i < n; i = i + 1) { a[i] = x } return 0 }\nf([0], 1)\n",
        );
        let (am, wm, bm) = classify_params(&c, c.param_count as usize);
        assert_eq!(
            (bit0(am), bit0(wm), bit0(bm)),
            (1, 1, 0),
            "number-local write"
        );

        // Mixed bool + number writes to the SAME array ⇒ the whole function is rejected.
        let c = arr_fn("function f(a, n) { a[0] = true; a[1] = 5; return 0 }\nf([0, 0], 1)\n");
        assert_eq!(
            classify_params(&c, c.param_count as usize),
            (0, 0, 0),
            "mixed bool+number writes must bail"
        );
    }

    /// Regression for the address-reuse stale hit (#247): compile one function, then overwrite the SAME
    /// heap `Chunk` (same address = the cache key) with a *different* function — what a long-lived
    /// process (REPL / multi-script embedder) does when a freed chunk address is reused. Before the
    /// fingerprint check the cache returned the first function's native code for the second.
    #[test]
    fn jit_cache_detects_address_reuse() {
        let mut boxed: Box<Chunk> = Box::new(fn_chunk("const f = (a, b) => a - b\nf(0, 0)\n"));
        let sub = try_compile_numeric(&boxed).expect("a - b must JIT");
        assert_eq!(sub.call(&[10.0, 3.0]), 7.0, "sub baseline");

        *boxed = fn_chunk("const f = (a, b) => a * b\nf(0, 0)\n"); // same address, different content
        let mul = try_compile_numeric(&boxed).expect("a * b must JIT");
        assert_eq!(
            mul.call(&[10.0, 3.0]),
            30.0,
            "stale cache hit: reused address returned the old (sub) fn instead of mul"
        );
    }

    /// Permanent guard for #168: shifts must (a) actually JIT-compile and (b) be bit-exact with the
    /// VM's `eval_binop` (`to_int32`/`to_uint32` + wrapping shift). Breaking either fails this test.
    #[test]
    fn jit_lowers_shifts_bit_exact_to_vm() {
        let shl = jit_arity2("const f = (a, b) => a << b\nf(0, 0)\n");
        let shr = jit_arity2("const f = (a, b) => a >> b\nf(0, 0)\n");
        let ushr = jit_arity2("const f = (a, b) => a >>> b\nf(0, 0)\n");
        let cases = [
            (1.0, 4.0),
            (1.0, 32.0),
            (1.0, 33.0),
            (1.0, -1.0),
            (-8.0, 1.0),
            (-1.0, 0.0),
            (-2.0, 1.0),
            (4294967295.0, 0.0),
            (3.9, 0.0),
            (4294967297.0, 0.0),
            (-123456789.0, 5.0),
            (65535.0, 16.0),
        ];
        for (a, b) in cases {
            assert_eq!(
                shl.call(&[a, b]),
                to_int32(a).wrapping_shl(to_uint32(b)) as f64,
                "<< {a} {b}"
            );
            assert_eq!(
                shr.call(&[a, b]),
                to_int32(a).wrapping_shr(to_uint32(b)) as f64,
                ">> {a} {b}"
            );
            assert_eq!(
                ushr.call(&[a, b]),
                to_uint32(a).wrapping_shr(to_uint32(b)) as f64,
                ">>> {a} {b}"
            );
        }
    }

    /// Top-level chunk of compiled `src` (where hot loops live) — #190 OSR targets these.
    fn top_chunk(src: &str) -> Chunk {
        let prog = tishlang_parser::parse(src).expect("parse");
        let opt = tishlang_opt::optimize(&prog);
        tishlang_bytecode::compile(&opt).expect("compile")
    }

    /// `(header_ip, region_end)` of the FIRST loop in `chunk` — its first `JumpBack` names the region
    /// the OSR trigger would compile.
    fn first_region(chunk: &Chunk) -> (usize, usize) {
        let code = &chunk.code;
        let mut ip = 0;
        while ip < code.len() {
            let op = Opcode::from_u8(code[ip]).expect("op");
            if op == Opcode::JumpBack {
                let dist = peek_u16(code, ip + 1).unwrap() as usize;
                let region_end = ip + 3;
                return (region_end - dist, region_end);
            }
            ip += op.instruction_size(code, ip).unwrap_or(1);
        }
        panic!("no JumpBack (loop) in chunk");
    }

    /// #190 — the region compiler + `LoopFn` ABI run a real numeric loop end-to-end (independent of
    /// the VM dispatch loop): `while (i < 10) { s = s + i; i = i + 1 }` from all-zero live-ins must
    /// leave `s = 45`, `i = 10` in the live slots and return an in-range exit id, without deopting.
    #[test]
    fn osr_region_runs_numeric_loop() {
        let chunk =
            top_chunk("let s = 0.0\nlet i = 0.0\nwhile (i < 10.0) { s = s + i; i = i + 1.0 }\n");
        let (header, end) = first_region(&chunk);
        let lf = try_compile_loop(&chunk, header, end).expect("pure-numeric loop must OSR-compile");
        assert!(!lf.used_slots.is_empty() && !lf.exits.is_empty());
        let mut buf = vec![0.0f64; lf.used_slots.len()];
        let mut deopt = 0u8;
        let exit = lf.call(&mut buf, &mut deopt);
        assert!((exit as usize) < lf.exits.len(), "exit id in range");
        assert_eq!(deopt, 0, "v1 region never sets the deopt flag");
        let mut got = buf.clone();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            got,
            vec![10.0, 45.0],
            "s=45 (sum 0..9), i=10 after the loop"
        );
    }

    /// #190 — a loop that touches a non-slot value (a general call) is not pure-numeric slot math, so
    /// the region must be rejected (negative-cached) and the VM keeps interpreting. `Math.max` is a
    /// 2-arg call (NOT the #186 unary intrinsic), so it stays a `Call` the region must reject.
    #[test]
    fn osr_region_rejects_calls() {
        let chunk = top_chunk(
            "let a = 0.0\nfor (let i = 0; i < 100; i = i + 1) { a = a + Math.max(i, 2.0) }\n",
        );
        let (header, end) = first_region(&chunk);
        assert!(
            try_compile_loop(&chunk, header, end).is_none(),
            "a loop containing a general call must not OSR-compile"
        );
    }

    /// #190 — a loop with nested branches compiles and computes correctly through multiple blocks:
    /// `while (i < 20) { if (i % 2 == 0) s = s + i; i = i + 1 }` → s = sum of evens in 0..19 = 90.
    #[test]
    fn osr_region_handles_branches() {
        let chunk = top_chunk(
            "let s = 0.0\nlet i = 0.0\nwhile (i < 20.0) { if (i % 2.0 === 0.0) { s = s + i }; i = i + 1.0 }\n",
        );
        let (header, end) = first_region(&chunk);
        let lf =
            try_compile_loop(&chunk, header, end).expect("branchy numeric loop must OSR-compile");
        let mut buf = vec![0.0f64; lf.used_slots.len()];
        let mut deopt = 0u8;
        lf.call(&mut buf, &mut deopt);
        let mut got = buf.clone();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(got, vec![20.0, 90.0], "s=90 (0+2+…+18), i=20");
    }
}
