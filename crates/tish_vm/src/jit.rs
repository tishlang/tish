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
    AbiParam, Block, FloatCC, FunctionBuilder, FunctionBuilderContext, InstBuilder,
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
                _ => f64::NAN,
            }
        }
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
        || chunk.param_count > 3
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
    let result_bool = match build_body_cfg(&mut ctx.func, &mut fbctx, chunk, arity, Some(self_ref)) {
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
    })
}

/// Outcome of trying to emit one *straight-line* numeric opcode.
enum SimpleOp {
    /// Handled: bytecode consumed, IR emitted, `bool` flags a comparison/`!` result.
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
                // Bitwise AND/OR/XOR. The VM does `((a as i32) OP (b as i32)) as f64`;
                // Rust's `f64 as i32` is *saturating*, which `fcvt_to_sint_sat` matches
                // exactly. Shifts / `>>>` stay on the VM (shift-amount edge cases).
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    let li = bcx.ins().fcvt_to_sint_sat(types::I32, l);
                    let ri = bcx.ins().fcvt_to_sint_sat(types::I32, r);
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
                // `~x` = `!(x as i32) as f64` (matches the VM; saturating cast).
                UnaryOp::BitNot => {
                    let oi = bcx.ins().fcvt_to_sint_sat(types::I32, o);
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
        Nop | Pop | Dup | Return | LoopVarsEnd | EnterBlock | ExitBlock => 1,
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

    // 2. A Variable per slot, all defined at entry (params, else 0.0) so every path defines them.
    let vars: Vec<Variable> = (0..num_slots).map(|_| bcx.declare_var(types::F64)).collect();
    for (i, &v) in vars.iter().enumerate() {
        let init = if i < arity {
            params[i]
        } else {
            bcx.ins().f64const(0.0)
        };
        bcx.def_var(v, init);
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
