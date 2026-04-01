use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::{ObjectMap, Value};

/// Called by the Tish runtime to get the module's exported functions.
pub fn metal_object() -> Value {
    let mut map = ObjectMap::default();
    map.insert(Arc::from("device_name"), Value::Function(Rc::new(|_: &[Value]| device_name())));
    map.insert(Arc::from("matmul_f32"),  Value::Function(Rc::new(|args: &[Value]| matmul_f32(args))));
    map.insert(Arc::from("run_f32"),     Value::Function(Rc::new(|args: &[Value]| run_f32(args))));
    Value::Object(Rc::new(RefCell::new(map)))
}

// ── non-macOS stubs ──────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
fn device_name() -> Value {
    Value::String("tish:metal is only available on macOS".into())
}
#[cfg(not(target_os = "macos"))]
fn matmul_f32(_args: &[Value]) -> Value {
    Value::String("tish:metal is only available on macOS".into())
}
#[cfg(not(target_os = "macos"))]
fn run_f32(_args: &[Value]) -> Value {
    Value::String("tish:metal is only available on macOS".into())
}

// ── macOS implementation ─────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use metal::*;
    use objc::rc::autoreleasepool;
    use std::time::Instant;

    const MATMUL_SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;
#define TILE 16
kernel void matmul_f32(
    device const float* A [[ buffer(0) ]],
    device const float* B [[ buffer(1) ]],
    device       float* C [[ buffer(2) ]],
    constant     uint&  N [[ buffer(3) ]],
    uint2 gid [[ thread_position_in_grid        ]],
    uint2 lid [[ thread_position_in_threadgroup ]])
{
    threadgroup float tA[TILE][TILE];
    threadgroup float tB[TILE][TILE];
    uint row = gid.y, col = gid.x;
    float acc = 0.0f;
    for (uint t = 0; t < (N + TILE - 1) / TILE; ++t) {
        uint aCol = t * TILE + lid.x, bRow = t * TILE + lid.y;
        tA[lid.y][lid.x] = (row < N && aCol < N) ? A[row * N + aCol] : 0.0f;
        tB[lid.y][lid.x] = (bRow < N && col < N) ? B[bRow * N + col] : 0.0f;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint k = 0; k < TILE; ++k) acc += tA[lid.y][k] * tB[k][lid.x];
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    if (row < N && col < N) C[row * N + col] = acc;
}
"#;

    fn make_result(ms: f64, check: f64) -> Value {
        let mut map = ObjectMap::default();
        map.insert(Arc::from("ms"),    Value::Number(ms));
        map.insert(Arc::from("check"), Value::Number(check));
        Value::Object(Rc::new(RefCell::new(map)))
    }

    pub fn device_name() -> Value {
        autoreleasepool(|| match Device::system_default() {
            Some(d) => Value::String(d.name().into()),
            None    => Value::String("no Metal device".into()),
        })
    }

    pub fn matmul_f32(args: &[Value]) -> Value {
        let n = match args.first() {
            Some(Value::Number(n)) => *n as usize,
            _ => return Value::String("matmul_f32: expected number".into()),
        };
        autoreleasepool(|| {
            let device   = match Device::system_default() { Some(d) => d, None => return Value::String("no Metal device".into()) };
            let queue    = device.new_command_queue();
            let lib      = device.new_library_with_source(MATMUL_SHADER, &CompileOptions::new()).unwrap();
            let func     = lib.get_function("matmul_f32", None).unwrap();
            let pipeline = device.new_compute_pipeline_state_with_function(&func).unwrap();

            let do_pass = |timed: bool, a_buf: &Buffer, b_buf: &Buffer, c_buf: &Buffer, n_buf: &Buffer| -> f64 {
                let t0 = if timed { Some(Instant::now()) } else { None };
                let cb  = queue.new_command_buffer();
                let enc = cb.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&pipeline);
                enc.set_buffer(0, Some(a_buf), 0);
                enc.set_buffer(1, Some(b_buf), 0);
                enc.set_buffer(2, Some(c_buf), 0);
                enc.set_buffer(3, Some(n_buf), 0);
                enc.dispatch_threads(
                    MTLSize::new(((n + 15) / 16) as _, ((n + 15) / 16) as _, 1),
                    MTLSize::new(16, 16, 1),
                );
                enc.end_encoding();
                cb.commit();
                cb.wait_until_completed();
                t0.map(|t| t.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0)
            };

            let bytes = (n * n * std::mem::size_of::<f32>()) as u64;
            let opts  = MTLResourceOptions::StorageModeShared;
            let a_buf = device.new_buffer(bytes, opts);
            let b_buf = device.new_buffer(bytes, opts);
            let c_buf = device.new_buffer(bytes, opts);
            unsafe {
                let a_ptr = a_buf.contents() as *mut f32;
                let b_ptr = b_buf.contents() as *mut f32;
                for i in 0..(n * n) {
                    *a_ptr.add(i) = (i % 997) as f32 / 997.0;
                    *b_ptr.add(i) = (i % 991) as f32 / 991.0;
                }
            }
            let n_u32 = n as u32;
            let n_buf = device.new_buffer_with_data(
                &n_u32 as *const u32 as *const _, std::mem::size_of::<u32>() as u64, opts);

            do_pass(false, &a_buf, &b_buf, &c_buf, &n_buf); // warm-up
            let ms = do_pass(true, &a_buf, &b_buf, &c_buf, &n_buf);

            let check = unsafe {
                let p = c_buf.contents() as *const f32;
                *p as f64 + *p.add(n-1) as f64 + *p.add((n-1)*n) as f64 + *p.add(n*n-1) as f64
            };
            make_result(ms, check)
        })
    }

    pub fn run_f32(args: &[Value]) -> Value {
        let shader  = match args.first() { Some(v) => v.to_display_string(), _ => return Value::String("run_f32: missing shader".into()) };
        let fn_name = match args.get(1)  { Some(v) => v.to_display_string(), _ => return Value::String("run_f32: missing fn_name".into()) };

        let inputs_owned: Vec<Vec<f32>> = match args.get(2).unwrap_or(&Value::Null) {
            Value::Array(outer) => outer.borrow().iter().map(|v| match v {
                Value::Array(inner) => inner.borrow().iter().filter_map(|x| match x {
                    Value::Number(n) => Some(*n as f32), _ => None,
                }).collect(),
                _ => vec![],
            }).collect(),
            _ => vec![],
        };

        let output_size = match args.get(3) { Some(Value::Number(n)) => *n as usize, _ => 0 };
        let gx  = match args.get(4) { Some(Value::Number(n)) => *n as usize, _ => output_size.max(1) };
        let gy  = match args.get(5) { Some(Value::Number(n)) => *n as usize, _ => 1 };
        let gz  = match args.get(6) { Some(Value::Number(n)) => *n as usize, _ => 1 };
        let tgx = match args.get(7) { Some(Value::Number(n)) => *n as usize, _ => gx.min(256).max(1) };
        let tgy = match args.get(8) { Some(Value::Number(n)) => *n as usize, _ => 1 };
        let tgz = match args.get(9) { Some(Value::Number(n)) => *n as usize, _ => 1 };

        autoreleasepool(|| {
            let device   = match Device::system_default() { Some(d) => d, None => return Value::String("no Metal device".into()) };
            let queue    = device.new_command_queue();
            let lib      = match device.new_library_with_source(&shader, &CompileOptions::new()) {
                Ok(l) => l, Err(e) => return Value::String(format!("MSL compile error: {e}").into()),
            };
            let func = match lib.get_function(&fn_name, None) {
                Ok(f) => f, Err(e) => return Value::String(format!("function '{fn_name}' not found: {e}").into()),
            };
            let pipeline = match device.new_compute_pipeline_state_with_function(&func) {
                Ok(p) => p, Err(e) => return Value::String(format!("pipeline error: {e}").into()),
            };

            let in_bufs: Vec<Buffer> = inputs_owned.iter().map(|data| {
                device.new_buffer_with_data(
                    data.as_ptr() as *const _,
                    (data.len() * std::mem::size_of::<f32>()) as u64,
                    MTLResourceOptions::StorageModeShared,
                )
            }).collect();
            let out_buf = device.new_buffer(
                (output_size * std::mem::size_of::<f32>()) as u64,
                MTLResourceOptions::StorageModeShared,
            );

            let cb  = queue.new_command_buffer();
            let enc = cb.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipeline);
            for (i, buf) in in_bufs.iter().enumerate() { enc.set_buffer(i as _, Some(buf), 0); }
            enc.set_buffer(in_bufs.len() as _, Some(&out_buf), 0);
            enc.dispatch_threads(
                MTLSize::new(gx as _, gy as _, gz as _),
                MTLSize::new(tgx as _, tgy as _, tgz as _),
            );
            enc.end_encoding();
            cb.commit();
            cb.wait_until_completed();

            let ptr = out_buf.contents() as *const f32;
            let vals: Vec<Value> = unsafe { std::slice::from_raw_parts(ptr, output_size) }
                .iter().map(|&f| Value::Number(f as f64)).collect();
            Value::Array(Rc::new(RefCell::new(vals)))
        })
    }
}

#[cfg(target_os = "macos")]
pub use imp::{device_name, matmul_f32, run_f32};
