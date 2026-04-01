//! Apple MLX matmul benchmark — standalone binary.
//! Uses mlx-rs (oxideai/mlx-rs), which vendors and builds the MLX C library
//! from source — only Xcode Command Line Tools required, no brew install.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("matmul-mlx requires macOS with Apple Silicon.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    use mlx_rs::{Array, ops::matmul};
    use std::time::Instant;

    let mlx_version = env!("TISH_MLX_RS_VERSION");
    println!("=== Apple MLX matmul (f32, Metal GPU) ===");
    println!("mlx-rs: {}   device: Metal (Apple Silicon)", mlx_version);

    for &n in &[256usize, 512, 1024, 2048, 4096] {
        let do_matmul = |n: usize| -> (f64, Vec<f32>) {
            // Deterministic f32 input — avoids the ambiguous random::uniform generics.
            let a_data: Vec<f32> = (0..n*n).map(|i| (i % 997) as f32 / 997.0).collect();
            let b_data: Vec<f32> = (0..n*n).map(|i| (i % 991) as f32 / 991.0).collect();
            let a = Array::from_slice(&a_data, &[n as i32, n as i32]);
            let b = Array::from_slice(&b_data, &[n as i32, n as i32]);

            let t0 = Instant::now();
            let c = matmul(&a, &b).unwrap();
            c.eval().unwrap();
            let ms = t0.elapsed().as_secs_f64() * 1000.0;

            let data: Vec<f32> = c.as_slice::<f32>().to_vec();
            (ms, data)
        };

        // Warm-up at target N — primes the MLX Metal JIT kernel for this shape.
        let _ = do_matmul(n);

        let (ms, data) = do_matmul(n);
        let check = data[0] as f64
            + data[n - 1] as f64
            + data[(n - 1) * n] as f64
            + data[n * n - 1] as f64;
        println!("mlx-metal  {}x{}  ms={:.4}  check={:.4}", n, n, ms, check);
    }
}
