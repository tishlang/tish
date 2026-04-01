use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::{ObjectMap, Value};

/// Called by the Tish runtime to get the module's exported functions.
pub fn mlx_object() -> Value {
    let mut map = ObjectMap::default();
    map.insert(Arc::from("device_name"), Value::Function(Rc::new(|_: &[Value]| device_name())));
    map.insert(Arc::from("version"),     Value::Function(Rc::new(|_: &[Value]| version())));
    map.insert(Arc::from("matmul_f32"),  Value::Function(Rc::new(|args: &[Value]| matmul_f32(args))));
    Value::Object(Rc::new(RefCell::new(map)))
}

// ── non-macOS stubs ──────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
fn device_name() -> Value { Value::String("tish:mlx is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn version()     -> Value { Value::String("n/a".into()) }
#[cfg(not(target_os = "macos"))]
fn matmul_f32(_: &[Value]) -> Value { Value::String("tish:mlx is only available on macOS".into()) }

// ── macOS implementation ─────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use mlx_rs::{Array, ops::matmul};
    use std::time::Instant;

    fn make_result(ms: f64, check: f64) -> Value {
        let mut map = ObjectMap::default();
        map.insert(Arc::from("ms"),    Value::Number(ms));
        map.insert(Arc::from("check"), Value::Number(check));
        Value::Object(Rc::new(RefCell::new(map)))
    }

    fn do_matmul(n: usize) -> Result<(f64, f64), String> {
        let a_data: Vec<f32> = (0..n*n).map(|i| (i % 997) as f32 / 997.0).collect();
        let b_data: Vec<f32> = (0..n*n).map(|i| (i % 991) as f32 / 991.0).collect();
        let a = Array::from_slice(&a_data, &[n as i32, n as i32]);
        let b = Array::from_slice(&b_data, &[n as i32, n as i32]);

        let t0 = Instant::now();
        let c = matmul(&a, &b).map_err(|e| e.to_string())?;
        c.eval().map_err(|e| e.to_string())?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;

        let data = c.as_slice::<f32>();
        let check = data[0] as f64 + data[n-1] as f64
            + data[(n-1)*n] as f64 + data[n*n-1] as f64;
        Ok((ms, check))
    }

    pub fn device_name() -> Value {
        Value::String("Metal (Apple Silicon)".into())
    }

    pub fn version() -> Value {
        Value::String("mlx-rs 0.25".into())
    }

    pub fn matmul_f32(args: &[Value]) -> Value {
        let n = match args.first() {
            Some(Value::Number(n)) => *n as usize,
            _ => return Value::String("matmul_f32: expected number".into()),
        };
        // Warm-up at target N — primes the MLX Metal JIT kernel for this shape.
        let _ = do_matmul(n);
        match do_matmul(n) {
            Ok((ms, check)) => make_result(ms, check),
            Err(e) => Value::String(e.into()),
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{device_name, version, matmul_f32};
