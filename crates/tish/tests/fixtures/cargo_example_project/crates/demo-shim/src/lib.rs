//! Rust exports for `import { … } from 'cargo:demo_shim'` (test fixture).

use std::sync::Arc;
use tishlang_core::Value;

pub fn greet(args: &[Value]) -> Value {
    let name = match args.first() {
        Some(Value::String(s)) => s.as_ref(),
        _ => "world",
    };
    Value::String(Arc::from(format!("Hello, {}!", name)))
}

/// Boxed shim kept for dynamic call sites (`Value::Function` dispatch / a spread call).
pub fn add(args: &[Value]) -> Value {
    let a = match args.first() {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    };
    let b = match args.get(1) {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    };
    Value::Number(a + b)
}

/// Typed extern (`declare fn add(a: number, b: number): number` in `tish.d.tish`): the direct, unboxed
/// entry codegen calls as `demo_shim::add_typed(..)` when the args lower to the declared native types.
/// `number` lowers to `f64` on every native target (unlike `i32`, which is native only under
/// `--target gba`), so this direct call fires on the default desktop build too.
pub fn add_typed(a: f64, b: f64) -> f64 {
    a + b
}
