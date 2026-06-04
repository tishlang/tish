//! Numeric JIT — slice 1 of native codegen.
//!
//! Compiles *straight-line numeric* `slot_based` functions (the self-contained
//! leaf callbacks RC2 already detects: `x => x * 2`, `(a, b) => a - b`,
//! `x => x === 500`, …) to native `f64`-in/`f64`-out machine code via Cranelift,
//! and the VM calls them directly from the `Call` path when every argument is a
//! number. Anything unsupported (control flow, member/index, calls, non-number
//! constants, mod/pow/bitwise, >3 params) makes compilation return `None`, and
//! any non-number argument at call time falls back to the interpreter — so this
//! is purely additive and can never change behaviour.
//!
//! Not compiled for wasm targets (cranelift-jit emits host code).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use cranelift::codegen::settings::{self, Configurable};
use cranelift::prelude::types;
use cranelift::prelude::{
    AbiParam, FloatCC, FunctionBuilder, FunctionBuilderContext, InstBuilder, Value as ClifValue,
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

    let mut ctx = g.module.make_context();
    let mut sig = g.module.make_signature();
    for _ in 0..arity {
        sig.params.push(AbiParam::new(types::F64));
    }
    sig.returns.push(AbiParam::new(types::F64));
    ctx.func.signature = sig.clone();

    let mut fbctx = FunctionBuilderContext::new();
    let built = build_body(&mut ctx.func, &mut fbctx, chunk, arity);
    if !built {
        g.module.clear_context(&mut ctx);
        return None;
    }

    let name = format!("tish_num_{}", g.counter);
    g.counter += 1;
    let id = match g.module.declare_function(&name, Linkage::Export, &sig) {
        Ok(id) => id,
        Err(_) => {
            g.module.clear_context(&mut ctx);
            return None;
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
    })
}

/// Translate the chunk's straight-line numeric bytecode into the function body.
/// Returns `false` (caller discards) on any unsupported opcode/operand.
fn build_body(
    func: &mut cranelift::codegen::ir::Function,
    fbctx: &mut FunctionBuilderContext,
    chunk: &Chunk,
    arity: usize,
) -> bool {
    let mut bcx = FunctionBuilder::new(func, fbctx);
    let entry = bcx.create_block();
    bcx.append_block_params_for_function_params(entry);
    bcx.switch_to_block(entry);
    bcx.seal_block(entry);
    let params: Vec<ClifValue> = bcx.block_params(entry).to_vec();

    let code = &chunk.code;
    let mut stack: Vec<ClifValue> = Vec::new();
    let mut ip = 0usize;
    let mut returned = false;

    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => return false,
        };
        ip += 1;
        match op {
            Opcode::Nop => {}
            Opcode::LoadLocal => {
                let slot = match read_u16(code, &mut ip) {
                    Some(s) => s as usize,
                    None => return false,
                };
                // Straight-line simple fns declare no locals, so only param slots exist.
                if slot >= arity {
                    return false;
                }
                stack.push(params[slot]);
            }
            Opcode::LoadConst => {
                let idx = match read_u16(code, &mut ip) {
                    Some(i) => i as usize,
                    None => return false,
                };
                match chunk.constants.get(idx) {
                    Some(Constant::Number(n)) => {
                        let v = bcx.ins().f64const(*n);
                        stack.push(v);
                    }
                    Some(Constant::Bool(b)) => {
                        let v = bcx.ins().f64const(if *b { 1.0 } else { 0.0 });
                        stack.push(v);
                    }
                    _ => return false,
                }
            }
            Opcode::BinOp => {
                let raw = match read_u16(code, &mut ip) {
                    Some(v) => v as u8,
                    None => return false,
                };
                let bop = match u8_to_binop(raw) {
                    Some(b) => b,
                    None => return false,
                };
                if stack.len() < 2 {
                    return false;
                }
                let r = stack.pop().unwrap();
                let l = stack.pop().unwrap();
                let v = match bop {
                    BinOp::Add => bcx.ins().fadd(l, r),
                    BinOp::Sub => bcx.ins().fsub(l, r),
                    BinOp::Mul => bcx.ins().fmul(l, r),
                    BinOp::Div => bcx.ins().fdiv(l, r),
                    BinOp::Eq | BinOp::StrictEq => fcmp_f64(&mut bcx, FloatCC::Equal, l, r),
                    BinOp::Ne | BinOp::StrictNe => fcmp_f64(&mut bcx, FloatCC::NotEqual, l, r),
                    BinOp::Lt => fcmp_f64(&mut bcx, FloatCC::LessThan, l, r),
                    BinOp::Le => fcmp_f64(&mut bcx, FloatCC::LessThanOrEqual, l, r),
                    BinOp::Gt => fcmp_f64(&mut bcx, FloatCC::GreaterThan, l, r),
                    BinOp::Ge => fcmp_f64(&mut bcx, FloatCC::GreaterThanOrEqual, l, r),
                    // Mod/Pow/bitwise/shifts/In/And/Or: not handled in slice 1.
                    _ => return false,
                };
                stack.push(v);
            }
            Opcode::UnaryOp => {
                let raw = match read_u16(code, &mut ip) {
                    Some(v) => v as u8,
                    None => return false,
                };
                let uop = match u8_to_unaryop(raw) {
                    Some(u) => u,
                    None => return false,
                };
                let o = match stack.pop() {
                    Some(v) => v,
                    None => return false,
                };
                let v = match uop {
                    UnaryOp::Neg => bcx.ins().fneg(o),
                    UnaryOp::Pos => o,
                    UnaryOp::Not => {
                        let zero = bcx.ins().f64const(0.0);
                        fcmp_f64(&mut bcx, FloatCC::Equal, o, zero)
                    }
                    _ => return false,
                };
                stack.push(v);
            }
            Opcode::Return => {
                let v = match stack.pop() {
                    Some(v) => v,
                    None => return false,
                };
                bcx.ins().return_(&[v]);
                returned = true;
                break; // straight-line: first Return ends the function
            }
            // Any control flow / member / index / call / array / object opcode
            // disqualifies the chunk from the numeric fast path.
            _ => return false,
        }
    }

    if !returned {
        bcx.finalize();
        return false;
    }
    bcx.finalize();
    true
}
