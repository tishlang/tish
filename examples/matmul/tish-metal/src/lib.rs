use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::{ObjectMap, Value};

/// Called by the Tish runtime to get the module's exported functions.
///
/// Exports:
///   device_name()                            → string
///   matmul(a, b, m, k, n)                   → number[]   a:[m×k], b:[k×n] → [m×n]
///   run_f32(shader, fn_name, inputs, output_size, gx, gy, gz, tgx, tgy, tgz) → number[]
///   run_i32(shader, fn_name, inputs, output_size, gx, gy, gz, tgx, tgy, tgz) → number[]
pub fn metal_object() -> Value {
    let mut map = ObjectMap::default();
    map.insert(Arc::from("device_name"), Value::Function(Rc::new(|_: &[Value]| device_name())));
    map.insert(Arc::from("matmul"),      Value::Function(Rc::new(|args: &[Value]| metal_matmul(args))));
    map.insert(Arc::from("run_f32"),     Value::Function(Rc::new(|args: &[Value]| run(args, false))));
    map.insert(Arc::from("run_i32"),     Value::Function(Rc::new(|args: &[Value]| run(args, true))));
    Value::Object(Rc::new(RefCell::new(map)))
}

// ── non-macOS stubs ──────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
fn device_name() -> Value { Value::String("tish:metal is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn metal_matmul(_: &[Value]) -> Value { Value::String("tish:metal is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn run(_args: &[Value], _i32_mode: bool) -> Value { Value::String("tish:metal is only available on macOS".into()) }

// ── macOS implementation ─────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use metal::*;
    use objc::rc::autoreleasepool;

    const GEMM_SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;
#define TILE 16
kernel void gemm(
    device const float* A   [[ buffer(0) ]],
    device const float* B   [[ buffer(1) ]],
    device const float* N_f [[ buffer(2) ]],
    device       float* C   [[ buffer(3) ]],
    uint2 gid [[ thread_position_in_grid        ]],
    uint2 lid [[ thread_position_in_threadgroup ]])
{
    uint N = (uint)N_f[0];
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

    /// Extract a flat f32 Vec from a Tish Value::Array of Value::Number.
    fn to_f32(v: &Value) -> Vec<f32> {
        match v {
            Value::Array(a) => a.borrow().iter().filter_map(|x| match x {
                Value::Number(n) => Some(*n as f32), _ => None,
            }).collect(),
            _ => vec![],
        }
    }

    pub fn device_name() -> Value {
        autoreleasepool(|| match Device::system_default() {
            Some(d) => Value::String(d.name().into()),
            None    => Value::String("no Metal device".into()),
        })
    }

    /// matmul(a, b, m, k, n) → number[]
    ///
    /// Multiply matrix a [m × k] by matrix b [k × n] on the Metal GPU.
    /// Both inputs are flat f32 arrays (row-major).
    /// Returns the flat [m × n] result.
    ///
    /// Same API as tish:mlx matmul — swap the import to switch backend.
    pub fn metal_matmul(args: &[Value]) -> Value {
        let a_data = to_f32(args.first().unwrap_or(&Value::Null));
        let b_data = to_f32(args.get(1).unwrap_or(&Value::Null));
        let m = match args.get(2) { Some(Value::Number(v)) => *v as usize, _ => return Value::String("matmul: missing m".into()) };
        let k = match args.get(3) { Some(Value::Number(v)) => *v as usize, _ => return Value::String("matmul: missing k".into()) };
        let n = match args.get(4) { Some(Value::Number(v)) => *v as usize, _ => return Value::String("matmul: missing n".into()) };

        if a_data.len() != m * k { return Value::String(format!("matmul: a has {} elements, expected m×k={}", a_data.len(), m*k).into()); }
        if b_data.len() != k * n { return Value::String(format!("matmul: b has {} elements, expected k×n={}", b_data.len(), k*n).into()); }

        // Square matmul only for the tiled 16×16 kernel (m==n==k is the common case).
        // For non-square, fall back to n for the grid size.
        let grid_x = (n + 15) / 16;
        let grid_y = (m + 15) / 16;
        let n_f = n as f32; // pass N (cols of B / cols of C) to the shader

        autoreleasepool(|| {
            let device = match Device::system_default() { Some(d) => d, None => return Value::String("no Metal device".into()) };
            let queue  = device.new_command_queue();
            let lib    = device.new_library_with_source(GEMM_SHADER, &CompileOptions::new()).unwrap();
            let func   = lib.get_function("gemm", None).unwrap();
            let pipeline = device.new_compute_pipeline_state_with_function(&func).unwrap();

            let opts = MTLResourceOptions::StorageModeShared;
            let a_buf = device.new_buffer_with_data(a_data.as_ptr() as *const _, (a_data.len() * 4) as u64, opts);
            let b_buf = device.new_buffer_with_data(b_data.as_ptr() as *const _, (b_data.len() * 4) as u64, opts);
            let n_buf = device.new_buffer_with_data(&n_f as *const f32 as *const _, 4, opts);
            let c_buf = device.new_buffer((m * n * 4) as u64, opts);

            let cb  = queue.new_command_buffer();
            let enc = cb.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipeline);
            enc.set_buffer(0, Some(&a_buf), 0);
            enc.set_buffer(1, Some(&b_buf), 0);
            enc.set_buffer(2, Some(&n_buf), 0);
            enc.set_buffer(3, Some(&c_buf), 0);
            enc.dispatch_threads(
                MTLSize::new(grid_x as _, grid_y as _, 1),
                MTLSize::new(16, 16, 1),
            );
            enc.end_encoding();
            cb.commit();
            cb.wait_until_completed();

            let ptr = c_buf.contents() as *const f32;
            let vals: Vec<Value> = unsafe { std::slice::from_raw_parts(ptr, m * n) }
                .iter().map(|&f| Value::Number(f as f64)).collect();
            Value::Array(Rc::new(RefCell::new(vals)))
        })
    }

    /// General-purpose Metal compute dispatch.
    ///
    /// run_f32 / run_i32 arguments (from Tish):
    ///   0  shader_src   string     MSL kernel source code
    ///   1  fn_name      string     kernel function name
    ///   2  inputs       number[][] one flat f32/i32 array per input buffer
    ///                              (mapped to buffer(0), buffer(1), …)
    ///   3  output_size  number     number of f32/i32 elements to return
    ///                              (mapped to the last buffer slot)
    ///   4  gx           number     grid x  (threads to dispatch in x)
    ///   5  gy           number     grid y
    ///   6  gz           number     grid z
    ///   7  tgx          number     threadgroup x
    ///   8  tgy          number     threadgroup y
    ///   9  tgz          number     threadgroup z
    ///
    /// Returns: number[] — the output buffer contents after the kernel runs.
    ///
    /// Example (Tish):
    ///   let result = run_f32(
    ///     `#include <metal_stdlib>
    ///      using namespace metal;
    ///      kernel void relu(device const float* a [[ buffer(0) ]],
    ///                       device       float* o [[ buffer(1) ]],
    ///                       uint gid [[ thread_position_in_grid ]]) {
    ///        o[gid] = a[gid] > 0.0f ? a[gid] : 0.0f;
    ///      }`,
    ///     "relu",
    ///     [[−3.0, −1.0, 0.0, 2.0, 5.0]],
    ///     5,
    ///     5, 1, 1,
    ///     5, 1, 1
    ///   )
    ///   // result → [0, 0, 0, 2, 5]
    pub fn run(args: &[Value], i32_mode: bool) -> Value {
        let shader  = match args.first() { Some(v) => v.to_display_string(), _ => return Value::String("run: missing shader".into()) };
        let fn_name = match args.get(1)  { Some(v) => v.to_display_string(), _ => return Value::String("run: missing fn_name".into()) };

        let inputs: Vec<Vec<f32>> = match args.get(2).unwrap_or(&Value::Null) {
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

        let elem = std::mem::size_of::<f32>(); // f32 and i32 are both 4 bytes

        autoreleasepool(|| {
            let device = match Device::system_default() {
                Some(d) => d,
                None => return Value::String("no Metal device".into()),
            };
            let queue = device.new_command_queue();
            let lib = match device.new_library_with_source(&shader, &CompileOptions::new()) {
                Ok(l) => l,
                Err(e) => return Value::String(format!("MSL compile error: {e}").into()),
            };
            let func = match lib.get_function(&fn_name, None) {
                Ok(f) => f,
                Err(e) => return Value::String(format!("function '{fn_name}' not found: {e}").into()),
            };
            let pipeline = match device.new_compute_pipeline_state_with_function(&func) {
                Ok(p) => p,
                Err(e) => return Value::String(format!("pipeline error: {e}").into()),
            };

            let opts = MTLResourceOptions::StorageModeShared;
            let in_bufs: Vec<Buffer> = inputs.iter().map(|data| {
                device.new_buffer_with_data(
                    data.as_ptr() as *const _,
                    (data.len() * elem) as u64,
                    opts,
                )
            }).collect();
            let out_buf = device.new_buffer((output_size * elem) as u64, opts);

            let cb  = queue.new_command_buffer();
            let enc = cb.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipeline);
            for (i, buf) in in_bufs.iter().enumerate() {
                enc.set_buffer(i as _, Some(buf), 0);
            }
            enc.set_buffer(in_bufs.len() as _, Some(&out_buf), 0);
            enc.dispatch_threads(
                MTLSize::new(gx as _, gy as _, gz as _),
                MTLSize::new(tgx as _, tgy as _, tgz as _),
            );
            enc.end_encoding();
            cb.commit();
            cb.wait_until_completed();

            if i32_mode {
                let ptr = out_buf.contents() as *const i32;
                let vals: Vec<Value> = unsafe { std::slice::from_raw_parts(ptr, output_size) }
                    .iter().map(|&v| Value::Number(v as f64)).collect();
                Value::Array(Rc::new(RefCell::new(vals)))
            } else {
                let ptr = out_buf.contents() as *const f32;
                let vals: Vec<Value> = unsafe { std::slice::from_raw_parts(ptr, output_size) }
                    .iter().map(|&v| Value::Number(v as f64)).collect();
                Value::Array(Rc::new(RefCell::new(vals)))
            }
        })
    }
}

#[cfg(target_os = "macos")]
pub fn device_name() -> Value { imp::device_name() }
#[cfg(target_os = "macos")]
pub fn metal_matmul(args: &[Value]) -> Value { imp::metal_matmul(args) }
#[cfg(target_os = "macos")]
pub fn run(args: &[Value], i32_mode: bool) -> Value { imp::run(args, i32_mode) }
