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
//! Anything unsupported (member/index, calls, arrays/objects, non-number constants, pow/shift,
//! booleans in slots, a ternary inside a loop) makes compilation return `None`, and any non-number
//! argument at call time falls back to the interpreter — so this is purely ADDITIVE and can never
//! change behaviour (a miss runs the VM). Only a logic bug here could, hence the differential
//! validation (vm-JIT ≡ interp ≡ node) + `tests/core/jit_loops.tish`.
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
    /// Keyed by the address of the (stable, un-cloned) nested `Chunk`. `None`
    /// caches "this chunk is not JIT-eligible" so we don't re-analyze it.
    cache: HashMap<usize, Option<NumericFn>>,
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
    let lock = jit()?;
    let mut g = lock.lock().ok()?;
    if let Some(cached) = g.cache.get(&key).copied() {
        return cached;
    }
    let result = compile_chunk(&mut g, chunk);
    g.cache.insert(key, result);
    result
}

/// Lower `f64` comparison to `1.0`/`0.0` (JS boolean-in-number form).
fn fcmp_f64(bcx: &mut FunctionBuilder, cc: FloatCC, a: ClifValue, b: ClifValue) -> ClifValue {
    let cond = bcx.ins().fcmp(cc, a, b);
    let one = bcx.ins().f64const(1.0);
    let zero = bcx.ins().f64const(0.0);
    bcx.ins().select(cond, one, zero)
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

    let mut sig = g.module.make_signature();
    for _ in 0..arity {
        sig.params.push(AbiParam::new(types::F64));
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
    let result_bool = match build_body_cfg(&mut ctx.func, &mut fbctx, chunk, arity, Some(self_ref), 0)
    {
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
    if build_body_cfg(&mut ctx.func, &mut fbctx, chunk, arity, None, mask).is_none() {
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
                // Bitwise AND/OR/XOR. The VM does `((a as i64 as i32) OP …) as f64` —
                // JS ToInt32 (modulo 2³²), so convert to I64 (saturating, exact for the
                // `< 2⁵³` values real code produces) then `ireduce` to I32 to take the low
                // 32 bits. Shifts / `>>>` stay on the VM (shift-amount edge cases).
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    let l64 = bcx.ins().fcvt_to_sint_sat(types::I64, l);
                    let r64 = bcx.ins().fcvt_to_sint_sat(types::I64, r);
                    let li = bcx.ins().ireduce(types::I32, l64);
                    let ri = bcx.ins().ireduce(types::I32, r64);
                    let res = match bop {
                        BinOp::BitAnd => bcx.ins().band(li, ri),
                        BinOp::BitOr => bcx.ins().bor(li, ri),
                        BinOp::BitXor => bcx.ins().bxor(li, ri),
                        _ => unreachable!(),
                    };
                    bcx.ins().fcvt_from_sint(types::F64, res)
                }
                // Pow/shifts/`>>>`/In/And/Or: fall back to the VM.
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
                // `~x` = `!(x as i64 as i32) as f64` — JS ToInt32 (modulo) via I64→ireduce,
                // matching the VM so a JIT-compiled `~` can't diverge on large values.
                UnaryOp::BitNot => {
                    let o64 = bcx.ins().fcvt_to_sint_sat(types::I64, o);
                    let oi = bcx.ins().ireduce(types::I32, o64);
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
    let blocks: std::collections::BTreeMap<usize, Block> =
        leaders.iter().map(|&o| (o, bcx.create_block())).collect();
    let entry = *blocks.get(&0)?;
    bcx.append_block_params_for_function_params(entry);
    bcx.switch_to_block(entry);
    let params: Vec<ClifValue> = bcx.block_params(entry).to_vec();

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
    let mut cur = entry;
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
