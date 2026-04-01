use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::{ObjectMap, Value};

/// Called by the Tish runtime to get the module's exported functions.
///
/// Exports:
///   device_name()                            → string
///   version()                                → string
///   matmul(a, b, m, k, n)                   → number[]   a:[m×k], b:[k×n] → [m×n]
///   add(a, b)                                → number[]   element-wise add
///   multiply(a, b)                           → number[]   element-wise multiply
///   relu(a)                                  → number[]   max(0, x)
///   softmax(a)                               → number[]   softmax over last axis
pub fn mlx_object() -> Value {
    let mut map = ObjectMap::default();
    map.insert(Arc::from("device_name"), Value::Function(Rc::new(|_: &[Value]| device_name())));
    map.insert(Arc::from("version"),     Value::Function(Rc::new(|_: &[Value]| version())));
    map.insert(Arc::from("matmul"),      Value::Function(Rc::new(|args: &[Value]| mlx_matmul(args))));
    map.insert(Arc::from("add"),         Value::Function(Rc::new(|args: &[Value]| mlx_add(args))));
    map.insert(Arc::from("multiply"),    Value::Function(Rc::new(|args: &[Value]| mlx_multiply(args))));
    map.insert(Arc::from("relu"),        Value::Function(Rc::new(|args: &[Value]| mlx_relu(args))));
    map.insert(Arc::from("softmax"),     Value::Function(Rc::new(|args: &[Value]| mlx_softmax(args))));
    Value::Object(Rc::new(RefCell::new(map)))
}

// ── non-macOS stubs ──────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
fn device_name()              -> Value { Value::String("tish:mlx is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn version()                  -> Value { Value::String("n/a".into()) }
#[cfg(not(target_os = "macos"))]
fn mlx_matmul(_: &[Value])   -> Value { Value::String("tish:mlx is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn mlx_add(_: &[Value])      -> Value { Value::String("tish:mlx is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn mlx_multiply(_: &[Value]) -> Value { Value::String("tish:mlx is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn mlx_relu(_: &[Value])     -> Value { Value::String("tish:mlx is only available on macOS".into()) }
#[cfg(not(target_os = "macos"))]
fn mlx_softmax(_: &[Value])  -> Value { Value::String("tish:mlx is only available on macOS".into()) }

// ── macOS implementation ─────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use mlx_rs::{Array, ops};

    /// Extract a flat f32 Vec from a Tish Value::Array of Value::Number.
    fn to_f32(v: &Value) -> Vec<f32> {
        match v {
            Value::Array(a) => a.borrow().iter().filter_map(|x| match x {
                Value::Number(n) => Some(*n as f32),
                _ => None,
            }).collect(),
            _ => vec![],
        }
    }

    /// Wrap a flat f32 slice back into a Tish Value::Array.
    fn from_f32(data: &[f32]) -> Value {
        let vals: Vec<Value> = data.iter().map(|&f| Value::Number(f as f64)).collect();
        Value::Array(Rc::new(RefCell::new(vals)))
    }

    /// Evaluate an MLX Array and read it back as a flat f32 slice.
    fn eval_to_vec(arr: &Array) -> Result<Vec<f32>, String> {
        arr.eval().map_err(|e| e.to_string())?;
        Ok(arr.as_slice::<f32>().to_vec())
    }

    pub fn device_name() -> Value {
        Value::String("Metal (Apple Silicon)".into())
    }

    pub fn version() -> Value {
        Value::String("mlx-rs 0.25".into())
    }

    /// matmul(a, b, m, k, n) → number[]
    ///
    /// Multiply matrix a [m × k] by matrix b [k × n].
    /// Both inputs are flat f32 arrays (row-major).
    /// Returns the flat [m × n] result.
    ///
    /// Example (Tish):
    ///   let a = [1.0, 2.0, 3.0, 4.0]   // 2×2
    ///   let b = [5.0, 6.0, 7.0, 8.0]   // 2×2
    ///   let c = matmul(a, b, 2, 2, 2)  // → [19, 22, 43, 50]
    pub fn mlx_matmul(args: &[Value]) -> Value {
        let a_data = to_f32(args.first().unwrap_or(&Value::Null));
        let b_data = to_f32(args.get(1).unwrap_or(&Value::Null));
        let m = match args.get(2) { Some(Value::Number(v)) => *v as i32, _ => return Value::String("matmul: missing m".into()) };
        let k = match args.get(3) { Some(Value::Number(v)) => *v as i32, _ => return Value::String("matmul: missing k".into()) };
        let n = match args.get(4) { Some(Value::Number(v)) => *v as i32, _ => return Value::String("matmul: missing n".into()) };

        if a_data.len() != (m * k) as usize { return Value::String(format!("matmul: a has {} elements, expected m×k={}", a_data.len(), m*k).into()); }
        if b_data.len() != (k * n) as usize { return Value::String(format!("matmul: b has {} elements, expected k×n={}", b_data.len(), k*n).into()); }

        let a = Array::from_slice(&a_data, &[m, k]);
        let b = Array::from_slice(&b_data, &[k, n]);
        match ops::matmul(&a, &b).and_then(|c| { c.eval()?; Ok(c) }) {
            Ok(c) => from_f32(c.as_slice::<f32>()),
            Err(e) => Value::String(e.to_string().into()),
        }
    }

    /// add(a, b) → number[]  — element-wise addition (same shape).
    pub fn mlx_add(args: &[Value]) -> Value {
        let a_data = to_f32(args.first().unwrap_or(&Value::Null));
        let b_data = to_f32(args.get(1).unwrap_or(&Value::Null));
        if a_data.len() != b_data.len() {
            return Value::String(format!("add: a.len={} != b.len={}", a_data.len(), b_data.len()).into());
        }
        let n = a_data.len() as i32;
        let a = Array::from_slice(&a_data, &[n]);
        let b = Array::from_slice(&b_data, &[n]);
        match ops::add(&a, &b).and_then(|c| { c.eval()?; Ok(c) }) {
            Ok(c) => from_f32(c.as_slice::<f32>()),
            Err(e) => Value::String(e.to_string().into()),
        }
    }

    /// multiply(a, b) → number[]  — element-wise multiplication (same shape).
    pub fn mlx_multiply(args: &[Value]) -> Value {
        let a_data = to_f32(args.first().unwrap_or(&Value::Null));
        let b_data = to_f32(args.get(1).unwrap_or(&Value::Null));
        if a_data.len() != b_data.len() {
            return Value::String(format!("multiply: a.len={} != b.len={}", a_data.len(), b_data.len()).into());
        }
        let n = a_data.len() as i32;
        let a = Array::from_slice(&a_data, &[n]);
        let b = Array::from_slice(&b_data, &[n]);
        match ops::multiply(&a, &b).and_then(|c| { c.eval()?; Ok(c) }) {
            Ok(c) => from_f32(c.as_slice::<f32>()),
            Err(e) => Value::String(e.to_string().into()),
        }
    }

    /// relu(a) → number[]  — max(0, x) element-wise.
    pub fn mlx_relu(args: &[Value]) -> Value {
        let a_data = to_f32(args.first().unwrap_or(&Value::Null));
        let n = a_data.len() as i32;
        let a = Array::from_slice(&a_data, &[n]);
        match mlx_rs::nn::relu(&a).and_then(|c| { c.eval()?; Ok(c) }) {
            Ok(c) => from_f32(c.as_slice::<f32>()),
            Err(e) => Value::String(e.to_string().into()),
        }
    }

    /// softmax(a) → number[]  — softmax over the last axis.
    pub fn mlx_softmax(args: &[Value]) -> Value {
        let a_data = to_f32(args.first().unwrap_or(&Value::Null));
        let n = a_data.len() as i32;
        let a = Array::from_slice(&a_data, &[n]);
        match ops::softmax_device(&a, None, mlx_rs::Stream::default()).and_then(|c| { c.eval()?; Ok(c) }) {
            Ok(c) => from_f32(c.as_slice::<f32>()),
            Err(e) => Value::String(e.to_string().into()),
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{device_name, version, mlx_matmul, mlx_add, mlx_multiply, mlx_relu, mlx_softmax};
