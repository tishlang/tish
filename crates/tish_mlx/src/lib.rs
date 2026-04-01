//! Apple MLX compute for Tish (Metal GPU via Apple's MLX framework).
//!
//! Exposes `matmul_f32(n)`, `device_name()`, and `version()` as plain Rust
//! functions so they can be wrapped by `tishlang_runtime` and `tish_eval`.
//!
//! MLX uses lazy evaluation — operations build a compute graph that is
//! dispatched to the Metal GPU when `eval()` is called.  Unified memory
//! means there is no CPU↔GPU copy overhead on Apple Silicon.
//!
//! Requirements:
//!   Apple Silicon Mac · macOS 14+
//!   Xcode Command Line Tools only — mlx-sys vendors the MLX C source and
//!   builds it from source via cmake. No brew/pip install needed.

// ── non-macOS stub ──────────────────────────────────────────────────────────
#[cfg(not(target_os = "macos"))]
pub fn matmul_f32(_n: usize) -> Result<(f64, f64), String> {
    Err("tish:mlx is only available on macOS (Apple Silicon)".into())
}

#[cfg(not(target_os = "macos"))]
pub fn device_name() -> String {
    "unavailable".into()
}

#[cfg(not(target_os = "macos"))]
pub fn version() -> String {
    "unavailable".into()
}

// ── macOS implementation ────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use mlx_rs::random;
    use std::time::Instant;

    /// Run an N×N f32 matrix multiply via MLX (Metal GPU).
    /// Returns `(elapsed_ms, corner_checksum)`.
    pub fn matmul_f32(n: usize) -> Result<(f64, f64), String> {
        let shape = [n as i32, n as i32];

        // Allocate and initialise — ops are lazy until eval().
        let a = random::uniform::<_, f32>(0.0f32, 1.0f32, &shape[..], None)
            .map_err(|e| format!("mlx uniform a: {e:?}"))?;
        let b = random::uniform::<_, f32>(0.0f32, 1.0f32, &shape[..], None)
            .map_err(|e| format!("mlx uniform b: {e:?}"))?;

        // Materialise A and B on the GPU before timing.
        a.eval().map_err(|e| format!("mlx eval a: {e:?}"))?;
        b.eval().map_err(|e| format!("mlx eval b: {e:?}"))?;

        let t0 = Instant::now();
        let c = a.matmul(&b)
            .map_err(|e| format!("mlx matmul: {e:?}"))?;
        c.eval().map_err(|e| format!("mlx eval c: {e:?}"))?; // GPU fence
        let ms = t0.elapsed().as_secs_f64() * 1000.0;

        // Read corner elements for a deterministic checksum.
        let data: &[f32] = c.as_slice();
        let check = data[0] as f64
            + data[n - 1] as f64
            + data[(n - 1) * n] as f64
            + data[n * n - 1] as f64;

        Ok((ms, check))
    }

    /// Human-readable device description (always Metal on Apple Silicon).
    pub fn device_name() -> String {
        "Metal (Apple Silicon)".into()
    }

    /// mlx-rs crate version (proxy for the underlying MLX library version).
    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[cfg(target_os = "macos")]
pub use imp::{device_name, matmul_f32, version};
