//! Metal GPU compute for Tish.
//!
//! Exposes `matmul_f32(n)` and `device_name()` as plain Rust functions
//! (no Tish `Value` types) so they can be wrapped by both `tishlang_runtime`
//! (compiled path) and `tish_eval` (interpreter path) independently.
//!
//! Requires macOS 13+ and Apple Silicon (or any Mac with a Metal-capable GPU).
//! The MSL kernel uses a 16×16 shared-memory tile for efficient GPU utilisation.

// ── non-macOS stub ──────────────────────────────────────────────────────────
#[cfg(not(target_os = "macos"))]
pub fn matmul_f32(_n: usize) -> Result<(f64, f64), String> {
    Err("tish:metal is only available on macOS".into())
}

#[cfg(not(target_os = "macos"))]
pub fn device_name() -> Option<String> {
    None
}

// ── macOS implementation ────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use metal::*;
    use objc::rc::autoreleasepool;
    use std::time::Instant;

    // Tiled 16×16 MSL matmul kernel (shared-memory for coalesced reads).
    const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

#define TILE 16

kernel void matmul_f32(
    device const float* A [[ buffer(0) ]],
    device const float* B [[ buffer(1) ]],
    device       float* C [[ buffer(2) ]],
    constant     uint&  N [[ buffer(3) ]],
    uint2 gid [[ thread_position_in_grid   ]],
    uint2 lid [[ thread_position_in_threadgroup ]])
{
    threadgroup float tA[TILE][TILE];
    threadgroup float tB[TILE][TILE];

    uint row = gid.y;
    uint col = gid.x;
    float acc = 0.0f;

    for (uint t = 0; t < (N + TILE - 1) / TILE; ++t) {
        uint aCol = t * TILE + lid.x;
        uint bRow = t * TILE + lid.y;
        tA[lid.y][lid.x] = (row < N && aCol < N) ? A[row * N + aCol] : 0.0f;
        tB[lid.y][lid.x] = (bRow < N && col < N) ? B[bRow * N + col] : 0.0f;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint k = 0; k < TILE; ++k)
            acc += tA[lid.y][k] * tB[k][lid.x];
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    if (row < N && col < N)
        C[row * N + col] = acc;
}
"#;

    /// Run an N×N f32 matrix multiply on the Metal GPU.
    /// Returns `(elapsed_ms, corner_checksum)`.
    pub fn matmul_f32(n: usize) -> Result<(f64, f64), String> {
        autoreleasepool(|| {
            let device = Device::system_default()
                .ok_or_else(|| "no Metal device — Apple Silicon required".to_string())?;

            let queue = device.new_command_queue();
            let opts  = CompileOptions::new();
            let lib   = device.new_library_with_source(SHADER, &opts)
                .map_err(|e| format!("MSL compile error: {e}"))?;
            let func  = lib.get_function("matmul_f32", None)
                .map_err(|e| format!("function lookup error: {e}"))?;
            let pipeline = device.new_compute_pipeline_state_with_function(&func)
                .map_err(|e| format!("pipeline error: {e}"))?;

            let bytes = (n * n * std::mem::size_of::<f32>()) as u64;
            let opts  = MTLResourceOptions::StorageModeShared;
            let a_buf = device.new_buffer(bytes, opts);
            let b_buf = device.new_buffer(bytes, opts);
            let c_buf = device.new_buffer(bytes, opts);

            // Fill A and B with deterministic f32 data.
            let a_ptr = a_buf.contents() as *mut f32;
            let b_ptr = b_buf.contents() as *mut f32;
            let c_ptr = c_buf.contents() as *mut f32;
            for i in 0..(n * n) {
                unsafe {
                    *a_ptr.add(i) = (i % 997) as f32 / 997.0;
                    *b_ptr.add(i) = (i % 991) as f32 / 991.0;
                    *c_ptr.add(i) = 0.0f32;
                }
            }

            let n_u32 = n as u32;
            let n_buf = device.new_buffer_with_data(
                &n_u32 as *const u32 as *const _,
                std::mem::size_of::<u32>() as u64,
                MTLResourceOptions::StorageModeShared,
            );

            // Warm-up pass (shader compilation is cached after the first call).
            {
                let cb  = queue.new_command_buffer();
                let enc = cb.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&pipeline);
                enc.set_buffer(0, Some(&a_buf), 0);
                enc.set_buffer(1, Some(&b_buf), 0);
                enc.set_buffer(2, Some(&c_buf), 0);
                enc.set_buffer(3, Some(&n_buf), 0);
                let tg   = MTLSize::new(16, 16, 1);
                let grid = MTLSize::new(
                    ((n + 15) / 16) as _,
                    ((n + 15) / 16) as _,
                    1,
                );
                enc.dispatch_threads(grid, tg);
                enc.end_encoding();
                cb.commit();
                cb.wait_until_completed();
            }

            // Timed pass.
            let t0 = Instant::now();
            {
                let cb  = queue.new_command_buffer();
                let enc = cb.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&pipeline);
                enc.set_buffer(0, Some(&a_buf), 0);
                enc.set_buffer(1, Some(&b_buf), 0);
                enc.set_buffer(2, Some(&c_buf), 0);
                enc.set_buffer(3, Some(&n_buf), 0);
                let tg   = MTLSize::new(16, 16, 1);
                let grid = MTLSize::new(
                    ((n + 15) / 16) as _,
                    ((n + 15) / 16) as _,
                    1,
                );
                enc.dispatch_threads(grid, tg);
                enc.end_encoding();
                cb.commit();
                cb.wait_until_completed();
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0;

            let check = unsafe {
                *c_ptr as f64
                    + *c_ptr.add(n - 1) as f64
                    + *c_ptr.add((n - 1) * n) as f64
                    + *c_ptr.add(n * n - 1) as f64
            };

            Ok((ms, check))
        })
    }

    /// Name of the default Metal device (e.g. "Apple M3 Pro").
    pub fn device_name() -> Option<String> {
        autoreleasepool(|| {
            Device::system_default().map(|d| d.name().to_string())
        })
    }
}

#[cfg(target_os = "macos")]
pub use imp::{device_name, matmul_f32};
