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
use std::sync::{Mutex, OnceLock};

use cranelift::codegen::settings::{self, Configurable};
use cranelift::prelude::types;
use cranelift::prelude::{
    AbiParam, Block, FloatCC, FunctionBuilder, FunctionBuilderContext, InstBuilder, IntCC, MemFlags,
    Value as ClifValue, Variable,
};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use tishlang_ast::{BinOp, UnaryOp};
use tishlang_bytecode::{u8_to_binop, u8_to_unaryop, Chunk, Constant, Opcode, NO_REST_PARAM};

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
    /// Run the region. `buf` points at `used_slots.len()` `f64`s (the live-ins), updated in place
    /// with the live-outs on return. `deopt` is a 1-byte flag (unused in v1). Returns the exit id.
    ///
    /// # Safety
    /// `buf` must be a valid, writable `[f64; used_slots.len()]` and `deopt` a valid `*mut u8`.
    #[inline]
    pub unsafe fn call(&self, buf: *mut f64, deopt: *mut u8) -> i32 {
        let f: extern "C" fn(*mut f64, *mut u8) -> i32 = std::mem::transmute(self.ptr);
        f(buf, deopt)
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
    /// True when this is a self-recursive function compiled with a trailing `*mut RecurGuard` param
    /// (the recursion-depth bail, #381). Such a function must be invoked via [`NumericFn::call_guarded`];
    /// non-recursive functions keep the plain ABI and [`NumericFn::call`] (zero overhead).
    recur_guarded: bool,
}

/// A flat numeric array handed to an array-mode JIT function: a raw `f64` slice (`ptr`, `len`).
/// Built by the VM wrapper from a `NumberArray` (zero-copy) or by extracting an all-numeric
/// `Array` into a scratch `Vec<f64>` (the wrapper only builds one when every element is a
/// `Value::Number` — non-numeric arrays never reach the JIT, so the slice is always valid `f64`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArrayHandle {
    pub ptr: *const f64,
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
    *ENABLED.get_or_init(|| std::env::var("TISH_JIT_ARRAYS").map(|v| v != "0").unwrap_or(true))
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
    *ENABLED
        .get_or_init(|| std::env::var("TISH_JIT_RECUR_GUARD").map(|v| v != "0").unwrap_or(true))
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
                    f(args[0], args[1], args[2], args[3], args[4], args[5], args[6])
                }
                8 => {
                    let f: extern "C" fn(f64, f64, f64, f64, f64, f64, f64, f64) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7])
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
                    let f: extern "C" fn(f64, *mut RecurGuard) -> f64 = std::mem::transmute(self.ptr);
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
                    let f: extern "C" fn(f64, f64, f64, f64, f64, f64, f64, *mut RecurGuard) -> f64 =
                        std::mem::transmute(self.ptr);
                    f(args[0], args[1], args[2], args[3], args[4], args[5], args[6], guard)
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
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], guard,
                    )
                }
                _ => f64::NAN,
            }
        }
    }

    /// Bit k set ⇒ param k is an array param (read via `arr[i]`). `0` ⇒ pure-numeric (use [`call`]).
    #[inline]
    pub fn array_param_mask(&self) -> u8 {
        self.array_param_mask
    }

    /// Call an array-mode function (`array_param_mask != 0`). `numeric` holds the f64 values for the
    /// numeric params in numeric-param order; `arrays` the [`ArrayHandle`]s for the array params in
    /// array-param order. Returns `(result, deopt)` — when `deopt` is true an out-of-bounds access
    /// was hit and the JIT bailed, so the caller MUST discard `result` and re-run the interpreter
    /// (OOB reads return `Value::Null` in the VM, whose per-operator coercion the JIT can't replicate).
    #[inline]
    pub fn call_arrays(&self, numeric: &[f64], arrays: &[ArrayHandle]) -> (f64, bool) {
        let mut deopt: u8 = 0;
        // ONE uniform signature for every array-mode fn: (numeric*, handles*, deopt*) -> f64. Empty
        // slices pass a dangling-but-aligned non-null ptr (the body only loads indices it uses).
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
            let f: extern "C" fn(*const f64, *const ArrayHandle, *mut u8) -> f64 =
                std::mem::transmute(self.ptr);
            f(num_ptr, arr_ptr, &mut deopt as *mut u8)
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
}

// SAFETY: `JITModule` is `!Send`, but the single instance lives behind the
// `Mutex` in the process-global `JIT` and is never moved out or dropped; all
// access is serialized by the mutex.
unsafe impl Send for JitGlobal {}

static JIT: OnceLock<Option<Mutex<JitGlobal>>> = OnceLock::new();

fn new_module() -> Option<JITModule> {
    let mut flag_builder = settings::builder();
    // JIT code is loaded at a fixed address; no PIC / colocated libcalls needed.
    flag_builder.set("use_colocated_libcalls", "false").ok()?;
    flag_builder.set("is_pic", "false").ok()?;
    flag_builder.set("opt_level", "speed").ok()?;
    let isa_builder = cranelift_native::builder().ok()?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .ok()?;
    let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    Some(JITModule::new(builder))
}

fn jit() -> Option<&'static Mutex<JitGlobal>> {
    JIT.get_or_init(|| {
        new_module().map(|module| {
            Mutex::new(JitGlobal {
                module,
                cache: HashMap::new(),
                osr_cache: HashMap::new(),
                counter: 0,
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
    g.cache.insert(key, (fp, result));
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
            | Opcode::UnaryOp => {}
            Opcode::LoadLocal | Opcode::StoreLocal => {
                used.insert(peek_u16(code, ip + 1)?);
            }
            Opcode::LoadConst => match chunk.constants.get(peek_u16(code, ip + 1)? as usize) {
                Some(Constant::Number(_)) | Some(Constant::Bool(_)) => {}
                _ => return None, // a String/Null/Closure const is not numeric slot math
            },
            Opcode::BinOp => {
                // Reject non-numeric binops up front (Pow/In/logical) so the scan and the emit agree.
                match peek_u16(code, ip + 1).map(|r| r as u8).and_then(u8_to_binop)? {
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
    let id = g.module.declare_function(&name, Linkage::Export, &sig).ok()?;

    let mut ctx = g.module.make_context();
    ctx.func.signature = sig.clone();
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
) -> bool {
    let code = &chunk.code;
    let num_slots = chunk.num_slots as usize;
    let mut bcx = FunctionBuilder::new(func, fbctx);

    let blocks: HashMap<usize, Block> =
        leaders.iter().map(|&o| (o, bcx.create_block())).collect();
    let exit_blocks: HashMap<usize, Block> =
        exit_id.keys().map(|&t| (t, bcx.create_block())).collect();

    // A jump target is either an in-region leader or a loop-exit target (the scan proved it is one).
    let target_block = |t: usize| -> Option<Block> {
        blocks.get(&t).or_else(|| exit_blocks.get(&t)).copied()
    };

    // Entry: load the live-in slots from the buffer, then jump into the loop header.
    let entry = bcx.create_block();
    bcx.append_block_params_for_function_params(entry);
    bcx.switch_to_block(entry);
    let params: Vec<ClifValue> = bcx.block_params(entry).to_vec();
    let slots_ptr = params[0]; // params[1] = deopt flag, reserved (v1 never writes it)
    let vars: Vec<Variable> = (0..num_slots).map(|_| bcx.declare_var(types::F64)).collect();
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
    let mut stack: Vec<(ClifValue, bool)> = Vec::new();
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
                stack.push((bcx.use_var(v), false));
                ip += 3;
            }
            Opcode::StoreLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return false,
                };
                let (val, is_bool) = match stack.pop() {
                    Some(x) => x,
                    None => return false,
                };
                if is_bool {
                    return false; // no boolean slots (keeps the number/bool distinction clean)
                }
                let v = match vars.get(slot) {
                    Some(v) => *v,
                    None => return false,
                };
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
                let (cond, _) = match stack.pop() {
                    Some(x) => x,
                    None => return false,
                };
                if !stack.is_empty() {
                    return false;
                }
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

/// Lower `f64` comparison to `1.0`/`0.0` (JS boolean-in-number form).
fn fcmp_f64(bcx: &mut FunctionBuilder, cc: FloatCC, a: ClifValue, b: ClifValue) -> ClifValue {
    let cond = bcx.ins().fcmp(cc, a, b);
    let one = bcx.ins().f64const(1.0);
    let zero = bcx.ins().f64const(0.0);
    bcx.ins().select(cond, one, zero)
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

fn compile_chunk(g: &mut JitGlobal, chunk: &Chunk) -> Option<NumericFn> {
    let arity = chunk.param_count as usize;

    // Array-mode (`TISH_JIT_ARRAYS`): if a param is used purely as `arr[i]`/`arr[const]`, compile the
    // 3-pointer array ABI instead of the register-`f64` ABI. mask 0 ⇒ ordinary numeric path below.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let array_mask = if jit_arrays_enabled() {
            classify_params(chunk, arity)
        } else {
            0
        };
        if array_mask != 0 {
            return compile_chunk_arrays(g, chunk, arity, array_mask);
        }
    }

    // #381: a self-recursive numeric function recurses on the native stack (SelfCall → a native call).
    // Compile it with a trailing `*mut RecurGuard` param so its entry can bail before the stack
    // overflows; non-recursive functions keep the plain register-f64 ABI (no param, no overhead).
    // Gated by `TISH_JIT_RECUR_GUARD` (default ON) so trusted hot-recursion workloads can trade the
    // guard's per-call cost for raw speed — see [`jit_recur_guard_enabled`].
    let recur_guard = jit_recur_guard_enabled() && chunk_has_self_call(chunk);

    let mut sig = g.module.make_signature();
    for _ in 0..arity {
        sig.params.push(AbiParam::new(types::F64));
    }
    if recur_guard {
        sig.params.push(AbiParam::new(g.module.target_config().pointer_type())); // *mut RecurGuard
    }
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
    let mut fbctx = FunctionBuilderContext::new();
    let result_bool = match build_body_cfg(
        &mut ctx.func,
        &mut fbctx,
        chunk,
        arity,
        Some(self_ref),
        0,
        recur_guard,
    ) {
        Some(b) => b,
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
    Some(NumericFn {
        ptr: ptr as usize,
        arity: arity as u8,
        result_bool,
        array_param_mask: 0,
        recur_guarded: recur_guard,
    })
}

/// Classify each param slot of an array-mode candidate. Returns a bitmask: **bit k set ⇒ param k is
/// an ARRAY** used only as `arr[i]` / `arr[const]`. Returns `0` when there are no array params OR the
/// function is ineligible for array mode (a param used both as an array and as a number, a `GetIndex`
/// the peephole can't consume, etc.) — the caller then takes the ordinary numeric path, which itself
/// bails on `GetIndex`, so a `0` here is always safe (never a miscompile, just no array-JIT).
#[cfg(not(target_arch = "wasm32"))]
fn classify_params(chunk: &Chunk, arity: usize) -> u8 {
    if arity == 0 || arity > 8 || (chunk.num_slots as usize) == 0 {
        return 0;
    }
    let code = &chunk.code;
    let mut used_numeric = [false; 8];
    let mut used_array = [false; 8];
    let mut ip = 0usize;
    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => return 0,
        };
        let size = match op_size(op) {
            Some(s) => s,
            None => return 0, // an opcode the array CFG can't handle ⇒ ineligible
        };
        match op {
            Opcode::LoadLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return 0,
                };
                // Peephole: `LoadLocal(arr) ; (LoadLocal|LoadConst) ; GetIndex` ⇒ array access of arr.
                let idx_then_getindex = matches!(
                    (
                        code.get(ip + 3).copied().and_then(Opcode::from_u8),
                        code.get(ip + 6).copied().and_then(Opcode::from_u8),
                    ),
                    (Some(Opcode::LoadLocal), Some(Opcode::GetIndex))
                        | (Some(Opcode::LoadConst), Some(Opcode::GetIndex))
                );
                if idx_then_getindex && slot < arity {
                    used_array[slot] = true;
                    ip += 7; // consume LoadLocal(arr) + index op + GetIndex
                    continue;
                } else if slot < arity {
                    used_numeric[slot] = true;
                }
            }
            Opcode::StoreLocal => {
                let slot = match peek_u16(code, ip + 1) {
                    Some(s) => s as usize,
                    None => return 0,
                };
                if slot < arity {
                    used_numeric[slot] = true; // a written param is numeric-shaped (can't be our array)
                }
            }
            // Any `GetIndex` not already consumed by the peephole above ⇒ an index shape we don't
            // handle (e.g. `arr[i+1]`, `arr[brr[i]]`) ⇒ ineligible.
            Opcode::GetIndex => return 0,
            _ => {}
        }
        ip += size;
    }
    let mut mask = 0u8;
    for k in 0..arity {
        if used_array[k] {
            if used_numeric[k] {
                return 0; // used as BOTH array and number ⇒ ambiguous ⇒ bail
            }
            mask |= 1u8 << k;
        }
    }
    mask
}

/// Compile an array-mode function: numeric params + array params (read as `arr[i]`). Uses ONE uniform
/// ABI for every such function — `extern "C" fn(numeric: *const f64, handles: *const ArrayHandle,
/// deopt: *mut u8) -> f64` — so there is a single transmute (no per-arity explosion). Out-of-bounds
/// reads set `*deopt` and bail (the caller re-runs the interpreter); non-numeric arrays never reach
/// here (the VM wrapper only calls this when every element is a `Value::Number`).
#[cfg(not(target_arch = "wasm32"))]
fn compile_chunk_arrays(g: &mut JitGlobal, chunk: &Chunk, arity: usize, mask: u8) -> Option<NumericFn> {
    let ptr_ty = g.module.target_config().pointer_type();
    let mut sig = g.module.make_signature();
    sig.params.push(AbiParam::new(ptr_ty)); // numeric_ptr
    sig.params.push(AbiParam::new(ptr_ty)); // handles_ptr
    sig.params.push(AbiParam::new(ptr_ty)); // deopt_ptr
    sig.returns.push(AbiParam::new(types::F64));

    let name = format!("tish_arr_{}", g.counter);
    g.counter += 1;
    let id = g.module.declare_function(&name, Linkage::Export, &sig).ok()?;

    let mut ctx = g.module.make_context();
    ctx.func.signature = sig.clone();
    let mut fbctx = FunctionBuilderContext::new();
    // No self-call in array mode (recursive call would need the array signature) → pass None.
    if build_body_cfg(&mut ctx.func, &mut fbctx, chunk, arity, None, mask, false).is_none() {
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
        recur_guarded: false,
    })
}

/// Outcome of trying to emit one *straight-line* numeric opcode.
enum SimpleOp {
    /// Handled: bytecode consumed, IR emitted, `bool` flags a comparison/`!` result.
    #[allow(dead_code)] // reserved: the flag will carry a comparison/`!` result; currently always false
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
    stack: &mut Vec<(ClifValue, bool)>,
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
            stack.push((params[slot], false));
        }
        Opcode::LoadConst => {
            let idx = match read_u16(code, ip) {
                Some(i) => i as usize,
                None => return SimpleOp::Unsupported,
            };
            match chunk.constants.get(idx) {
                Some(Constant::Number(n)) => {
                    let v = bcx.ins().f64const(*n);
                    stack.push((v, false));
                }
                Some(Constant::Bool(b)) => {
                    let v = bcx.ins().f64const(if *b { 1.0 } else { 0.0 });
                    stack.push((v, true));
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
            let (r, _) = stack.pop().unwrap();
            let (l, _) = stack.pop().unwrap();
            let is_cmp = matches!(
                bop,
                BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe
                    | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
            );
            let v = match bop {
                BinOp::Add => bcx.ins().fadd(l, r),
                BinOp::Sub => bcx.ins().fsub(l, r),
                BinOp::Mul => bcx.ins().fmul(l, r),
                BinOp::Div => bcx.ins().fdiv(l, r),
                BinOp::Eq | BinOp::StrictEq => fcmp_f64(bcx, FloatCC::Equal, l, r),
                BinOp::Ne | BinOp::StrictNe => fcmp_f64(bcx, FloatCC::NotEqual, l, r),
                BinOp::Lt => fcmp_f64(bcx, FloatCC::LessThan, l, r),
                BinOp::Le => fcmp_f64(bcx, FloatCC::LessThanOrEqual, l, r),
                BinOp::Gt => fcmp_f64(bcx, FloatCC::GreaterThan, l, r),
                BinOp::Ge => fcmp_f64(bcx, FloatCC::GreaterThanOrEqual, l, r),
                BinOp::Mod => {
                    // f64 remainder a - trunc(a/b)*b — exactly Rust's `%`, which the
                    // VM's eval_binop uses, so JIT and VM-fallback agree bit-for-bit.
                    let q = bcx.ins().fdiv(l, r);
                    let t = bcx.ins().trunc(q);
                    let p = bcx.ins().fmul(t, r);
                    bcx.ins().fsub(l, p)
                }
                // Bitwise AND/OR/XOR via JS ToInt32 (modulo 2³², NaN/±∞ → 0) — see [`js_to_int32`].
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    let li = js_to_int32(bcx, l);
                    let ri = js_to_int32(bcx, r);
                    let res = match bop {
                        BinOp::BitAnd => bcx.ins().band(li, ri),
                        BinOp::BitOr => bcx.ins().bor(li, ri),
                        BinOp::BitXor => bcx.ins().bxor(li, ri),
                        _ => unreachable!(),
                    };
                    bcx.ins().fcvt_from_sint(types::F64, res)
                }
                // Shifts. JS masks the count to the low 5 bits (`& 31`); the low 5 bits of `ToInt32(r)`
                // equal `ToUint32(r)`, so `js_to_int32(r)` carries the right amount. We mask explicitly
                // (`& 31`) so correctness never depends on Cranelift's own amount-masking convention.
                // `<<`/`>>` are signed-domain (ToInt32 → i32 → signed→f64); `>>>` is logical on the
                // unsigned bits with an UNSIGNED→f64 convert (result may exceed 2³¹). Bit-for-bit with
                // vm.rs `eval_binop`: Shl/Shr = `to_int32(l).wrapping_sh*(to_uint32(r))`,
                // UShr = `to_uint32(l).wrapping_shr(to_uint32(r))`.
                BinOp::Shl | BinOp::Shr | BinOp::UShr => {
                    let li = js_to_int32(bcx, l);
                    let amt = js_to_int32(bcx, r);
                    let mask = bcx.ins().iconst(types::I32, 31);
                    let amt = bcx.ins().band(amt, mask);
                    match bop {
                        BinOp::Shl => {
                            let res = bcx.ins().ishl(li, amt);
                            bcx.ins().fcvt_from_sint(types::F64, res)
                        }
                        BinOp::Shr => {
                            let res = bcx.ins().sshr(li, amt);
                            bcx.ins().fcvt_from_sint(types::F64, res)
                        }
                        // UShr: logical shift on the same 32-bit value bits as ToUint32(l), then
                        // unsigned→f64 so a result with bit 31 set stays a positive number (JS `>>>`).
                        _ => {
                            let res = bcx.ins().ushr(li, amt);
                            bcx.ins().fcvt_from_uint(types::F64, res)
                        }
                    }
                }
                // Pow/In/And/Or: fall back to the VM.
                _ => return SimpleOp::Unsupported,
            };
            stack.push((v, is_cmp));
        }
        Opcode::UnaryOp => {
            let uop = match read_u16(code, ip).map(|r| r as u8).and_then(u8_to_unaryop) {
                Some(u) => u,
                None => return SimpleOp::Unsupported,
            };
            let (o, _) = match stack.pop() {
                Some(x) => x,
                None => return SimpleOp::Unsupported,
            };
            let (v, is_bool) = match uop {
                UnaryOp::Neg => (bcx.ins().fneg(o), false),
                UnaryOp::Pos => (o, false),
                UnaryOp::Not => {
                    let zero = bcx.ins().f64const(0.0);
                    (fcmp_f64(bcx, FloatCC::Equal, o, zero), true)
                }
                // `~x` = `!ToInt32(x) as f64` — JS ToInt32 (modulo, NaN/±∞ → 0) via [`js_to_int32`],
                // matching the VM so a JIT-compiled `~` can't diverge on large/non-finite values.
                UnaryOp::BitNot => {
                    let oi = js_to_int32(bcx, o);
                    let res = bcx.ins().bnot(oi);
                    (bcx.ins().fcvt_from_sint(types::F64, res), false)
                }
                _ => return SimpleOp::Unsupported,
            };
            stack.push((v, is_bool));
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
        Nop | Pop | Dup | Return | LoopVarsEnd | EnterBlock | ExitBlock | GetIndex => 1,
        LoadLocal | StoreLocal | LoadConst | BinOp | UnaryOp | Jump | JumpIfFalse | JumpBack
        | LoopVarsBegin | SelfCall => 3,
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

/// Read a big-endian u16 operand at `off` without advancing (matches [`read_u16`]).
#[inline]
fn peek_u16(code: &[u8], off: usize) -> Option<u16> {
    let a = *code.get(off)? as u16;
    let b = *code.get(off + 1)? as u16;
    Some((a << 8) | b)
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
) -> Option<bool> {
    let code = &chunk.code;
    let num_slots = chunk.num_slots as usize;
    if num_slots == 0 || num_slots > 256 {
        return None;
    }

    // 1. Validate every opcode is supported + collect block leaders (jump targets, the fall-through
    //    after each branch, entry). Bail on any unsupported opcode (so we never mis-size the scan).
    let mut leaders: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    leaders.insert(0);
    let mut has_loop = false;
    let mut has_self_call = false;
    let mut ip = 0;
    while ip < code.len() {
        let op = Opcode::from_u8(code[ip])?;
        let size = op_size(op)?;
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
    let guard_ptr: Option<ClifValue> = if recur_guard && array_mask == 0 {
        let gp = *params.get(arity)?;
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

    // 2. A Variable per slot, all defined at entry so every path defines them.
    let vars: Vec<Variable> = (0..num_slots).map(|_| bcx.declare_var(types::F64)).collect();
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
        #[allow(clippy::needless_range_loop)] // `slot` drives bit-mask math (`array_mask >> slot`) + map keys, not just indexing
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
            let init = if i < arity {
                params[i]
            } else {
                bcx.ins().f64const(0.0)
            };
            bcx.def_var(v, init);
        }
    }

    // 3. Translate. The operand stack is empty at every block boundary (statement-level control flow).
    let mut stack: Vec<(ClifValue, bool)> = Vec::new();
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
                bcx.switch_to_block(blk);
                cur = blk;
                terminated = false;
                stack.clear();
            }
        }
        if terminated {
            ip += op_size(Opcode::from_u8(code[ip])?)?; // skip unreachable tail before next leader
            continue;
        }
        let op = Opcode::from_u8(code[ip])?;
        match op {
            Opcode::LoadLocal => {
                let slot = peek_u16(code, ip + 1)? as usize;
                // Array-mode peephole: `LoadLocal(arrayparam) ; (LoadLocal|LoadConst) ; GetIndex` →
                // a bounds-checked native f64 load; out-of-bounds branches to the deopt pad.
                if array_mask != 0 && slot < arity && (array_mask >> slot) & 1 == 1 {
                    let (aptr, alen) = *array_slots.get(&slot)?;
                    let idx_op = Opcode::from_u8(*code.get(ip + 3)?)?;
                    let idx_f64 = match idx_op {
                        Opcode::LoadLocal => {
                            let islot = peek_u16(code, ip + 4)? as usize;
                            bcx.use_var(*vars.get(islot)?)
                        }
                        Opcode::LoadConst => {
                            let ci = peek_u16(code, ip + 4)? as usize;
                            match chunk.constants.get(ci) {
                                Some(Constant::Number(n)) => bcx.ins().f64const(*n),
                                _ => return None,
                            }
                        }
                        _ => return None,
                    };
                    if Opcode::from_u8(*code.get(ip + 6)?)? != Opcode::GetIndex {
                        return None;
                    }
                    // i = idx as usize (saturating: NaN→0, neg→0 — matches the VM's `n as usize`).
                    let i = bcx.ins().fcvt_to_uint_sat(types::I64, idx_f64);
                    let inb = bcx.ins().icmp(IntCC::UnsignedLessThan, i, alen);
                    let cont = bcx.create_block();
                    let db = deopt_block?;
                    bcx.ins().brif(inb, cont, &[], db, &[]);
                    bcx.switch_to_block(cont);
                    cur = cont; // keep block-boundary tracking accurate after the mid-stream split
                    let off = bcx.ins().imul_imm(i, 8);
                    let addr = bcx.ins().iadd(aptr, off);
                    let val = bcx.ins().load(types::F64, MemFlags::new(), addr, 0);
                    stack.push((val, false));
                    ip += 7; // LoadLocal(arr) + index op + GetIndex
                    continue;
                }
                let v = *vars.get(slot)?;
                stack.push((bcx.use_var(v), false));
                ip += 3;
            }
            Opcode::StoreLocal => {
                let slot = peek_u16(code, ip + 1)? as usize;
                let (val, is_bool) = stack.pop()?;
                if is_bool {
                    return None; // no boolean slots — keeps result boxing simple
                }
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
                let (v, is_bool) = stack.pop()?;
                if is_bool {
                    return None;
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
                let (cond, _) = stack.pop()?;
                if !stack.is_empty() {
                    return None; // non-empty stack ⇒ ternary shape ⇒ leave to build_body / VM
                }
                let falsy = falsy_flag(&mut bcx, cond);
                let target = *blocks.get(&(((ip + 3) as isize + off).max(0) as usize))?;
                let fallthrough = *blocks.get(&(ip + 3))?;
                bcx.ins().brif(falsy, target, &[], fallthrough, &[]);
                terminated = true;
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
                let mut call_args = Vec::with_capacity(arity);
                for (v, is_bool) in stack.drain(arg_start..) {
                    if is_bool {
                        return None; // boolean args don't match the f64 ABI
                    }
                    call_args.push(v);
                }
                // #381: thread the RecurGuard pointer through the recursive call so every level
                // re-checks the stack at its entry. Present iff this function was compiled guarded.
                if let Some(gp) = guard_ptr {
                    call_args.push(gp);
                }
                let call = bcx.ins().call(sref, &call_args);
                let result = bcx.inst_results(call)[0];
                stack.push((result, false));
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
    // Each entry is (clif f64 value, is_bool). `is_bool` marks comparison/`!`
    // results (logical 0.0/1.0) so the final value boxes as Bool, not Number.
    let mut stack: Vec<(ClifValue, bool)> = Vec::new();
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
                let (v, is_bool) = stack.pop()?;
                bcx.ins().return_(&[v]);
                result = Some(is_bool); // first Return ends a (sub)path
                break;
            }
            // Ternary `cond ? A : B` → `select`. Both arms must be branch-free numeric
            // sub-sequences, each pushing exactly one value, with matching is_bool.
            Opcode::JumpIfFalse => {
                let (cond, _) = stack.pop()?;
                let mut p = ip + 1;
                let off = read_u16(code, &mut p)? as i16 as isize; // p now past the operand
                let else_target = (p as isize + off).max(0) as usize;
                let base = stack.len();

                // THEN arm: straight-line ops until the trailing `Jump`.
                let mut tip = p;
                loop {
                    match emit_simple_op(&mut bcx, chunk, code, &mut tip, &mut stack, &params, arity)
                    {
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
                let (then_v, then_b) = stack.pop()?;

                // ELSE arm: straight-line ops from `jp` up to the merge point.
                let mut eip = jp;
                while eip < merge_target {
                    match emit_simple_op(&mut bcx, chunk, code, &mut eip, &mut stack, &params, arity)
                    {
                        SimpleOp::Handled(_) => continue,
                        _ => return None, // nested control flow / unsupported → VM
                    }
                }
                if eip != merge_target || stack.len() != base + 1 {
                    return None;
                }
                let (else_v, else_b) = stack.pop()?;
                // One result_bool per function: arms must agree on Bool-vs-Number.
                if then_b != else_b {
                    return None;
                }

                let falsy = falsy_flag(&mut bcx, cond);
                let sel = bcx.ins().select(falsy, else_v, then_v);
                stack.push((sel, then_b));
                ip = merge_target;
            }
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
            (1.0, 4.0), (1.0, 32.0), (1.0, 33.0), (1.0, -1.0), (-8.0, 1.0), (-1.0, 0.0),
            (-2.0, 1.0), (4294967295.0, 0.0), (3.9, 0.0), (4294967297.0, 0.0),
            (-123456789.0, 5.0), (65535.0, 16.0),
        ];
        for (a, b) in cases {
            assert_eq!(shl.call(&[a, b]), to_int32(a).wrapping_shl(to_uint32(b)) as f64, "<< {a} {b}");
            assert_eq!(shr.call(&[a, b]), to_int32(a).wrapping_shr(to_uint32(b)) as f64, ">> {a} {b}");
            assert_eq!(ushr.call(&[a, b]), to_uint32(a).wrapping_shr(to_uint32(b)) as f64, ">>> {a} {b}");
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
        let exit = unsafe { lf.call(buf.as_mut_ptr(), &mut deopt as *mut u8) };
        assert!((exit as usize) < lf.exits.len(), "exit id in range");
        assert_eq!(deopt, 0, "v1 region never sets the deopt flag");
        let mut got = buf.clone();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(got, vec![10.0, 45.0], "s=45 (sum 0..9), i=10 after the loop");
    }

    /// #190 — a loop that touches a non-slot value (a call) is not pure-numeric slot math, so the
    /// region must be rejected (negative-cached) and the VM keeps interpreting.
    #[test]
    fn osr_region_rejects_calls() {
        let chunk = top_chunk(
            "let a = 0.0\nfor (let i = 0; i < 100; i = i + 1) { a = a + Math.floor(i / 2.0) }\n",
        );
        let (header, end) = first_region(&chunk);
        assert!(
            try_compile_loop(&chunk, header, end).is_none(),
            "a loop containing a call must not OSR-compile"
        );
    }

    /// #190 — a loop with nested branches compiles and computes correctly through multiple blocks:
    /// `while (i < 20) { if (i % 2 == 0) s = s + i; i = i + 1 }` → s = sum of evens in 0..19 = 90.
    #[test]
    fn osr_region_handles_branches() {
        let chunk = top_chunk(
            "let s = 0.0\nlet i = 0.0\nwhile (i < 20.0) { if (i % 2.0 == 0.0) { s = s + i }; i = i + 1.0 }\n",
        );
        let (header, end) = first_region(&chunk);
        let lf = try_compile_loop(&chunk, header, end).expect("branchy numeric loop must OSR-compile");
        let mut buf = vec![0.0f64; lf.used_slots.len()];
        let mut deopt = 0u8;
        unsafe { lf.call(buf.as_mut_ptr(), &mut deopt as *mut u8) };
        let mut got = buf.clone();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(got, vec![20.0, 90.0], "s=90 (0+2+…+18), i=20");
    }
}
