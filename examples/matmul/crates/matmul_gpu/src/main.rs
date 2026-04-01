//! Metal GPU matmul benchmark — standalone binary.
//! Tiled 16×16 MSL compute kernel, warm-up + timed pass at each N.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("matmul-gpu requires macOS with a Metal GPU.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    use metal::*;
    use objc::rc::autoreleasepool;
    use std::time::Instant;

    const SHADER: &str = r#"
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

    autoreleasepool(|| {
        let device   = Device::system_default().expect("no Metal device");
        let queue    = device.new_command_queue();
        let lib      = device.new_library_with_source(SHADER, &CompileOptions::new()).unwrap();
        let func     = lib.get_function("matmul_f32", None).unwrap();
        let pipeline = device.new_compute_pipeline_state_with_function(&func).unwrap();

        println!("=== Metal GPU matmul (f32, tiled MSL compute) ===");
        println!("device: {}", device.name());

        for &n in &[512usize, 1024, 2048, 4096] {
            let bytes = (n * n * std::mem::size_of::<f32>()) as u64;
            let opts  = MTLResourceOptions::StorageModeShared;
            let a_buf = device.new_buffer(bytes, opts);
            let b_buf = device.new_buffer(bytes, opts);
            let c_buf = device.new_buffer(bytes, opts);

            let a_ptr = a_buf.contents() as *mut f32;
            let b_ptr = b_buf.contents() as *mut f32;
            unsafe {
                for i in 0..(n * n) {
                    *a_ptr.add(i) = (i % 997) as f32 / 997.0;
                    *b_ptr.add(i) = (i % 991) as f32 / 991.0;
                }
            }

            let n_u32 = n as u32;
            let n_buf = device.new_buffer_with_data(
                &n_u32 as *const u32 as *const _,
                std::mem::size_of::<u32>() as u64,
                MTLResourceOptions::StorageModeShared,
            );

            let dispatch = |timed: bool| -> f64 {
                let t0 = if timed { Some(Instant::now()) } else { None };
                let cb  = queue.new_command_buffer();
                let enc = cb.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&pipeline);
                enc.set_buffer(0, Some(&a_buf), 0);
                enc.set_buffer(1, Some(&b_buf), 0);
                enc.set_buffer(2, Some(&c_buf), 0);
                enc.set_buffer(3, Some(&n_buf), 0);
                enc.dispatch_threads(
                    MTLSize::new(((n + 15) / 16) as _, ((n + 15) / 16) as _, 1),
                    MTLSize::new(16, 16, 1),
                );
                enc.end_encoding();
                cb.commit();
                cb.wait_until_completed();
                t0.map(|t| t.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0)
            };

            dispatch(false); // warm-up at target N
            let ms = dispatch(true);

            let c_ptr = c_buf.contents() as *const f32;
            let check = unsafe {
                *c_ptr as f64 + *c_ptr.add(n - 1) as f64
                    + *c_ptr.add((n - 1) * n) as f64 + *c_ptr.add(n * n - 1) as f64
            };
            println!("metal  {}x{}  ms={:.4}  check={:.4}", n, n, ms, check);
        }
    });
}
