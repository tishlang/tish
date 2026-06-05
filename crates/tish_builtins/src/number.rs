//! Number builtin methods.
//!
//! Canonical, backend-agnostic implementations of `Number.prototype` methods.
//! The VM (`get_member`), the Rust runtime (`tishlang_runtime::number_to_fixed`),
//! and the tree-walk interpreter all route through here so every backend produces
//! byte-identical output — see `tish/docs/full-backend-parity-plan.md` (Workstream A).

use tishlang_core::Value;

/// `Number.prototype.toFixed(digits)` — ECMA-262 §21.1.3.3.
///
/// Formats the number using fixed-point notation with `digits` fraction digits.
/// `digits` is clamped to 0–20 (ECMA range) and defaults to 0 when absent/non-numeric,
/// matching `(1.5).toFixed() === "2"`. A non-number receiver yields `"NaN"`.
pub fn to_fixed(n: &Value, digits: &Value) -> Value {
    let num = match n {
        Value::Number(x) => *x,
        _ => f64::NAN,
    };
    let d = match digits {
        Value::Number(x) => (*x as i32).clamp(0, 20),
        _ => 0,
    } as usize;
    Value::String(format!("{:.*}", d, num).into())
}
