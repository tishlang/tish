//! Math builtin functions.

use tish_core::Value;
use crate::helpers::extract_num;

macro_rules! math_unary {
    ($name:ident, $op:ident) => {
        pub fn $name(args: &[Value]) -> Value {
            let n = extract_num(args.first()).unwrap_or(f64::NAN);
            Value::Number(n.$op())
        }
    };
}

math_unary!(abs, abs);
math_unary!(sqrt, sqrt);
math_unary!(floor, floor);
math_unary!(ceil, ceil);
math_unary!(round, round);
math_unary!(sin, sin);
math_unary!(cos, cos);
math_unary!(tan, tan);
math_unary!(asin, asin);
math_unary!(acos, acos);
math_unary!(atan, atan);
math_unary!(log, ln);
math_unary!(log10, log10);
math_unary!(log2, log2);
math_unary!(exp, exp);
math_unary!(trunc, trunc);
math_unary!(cbrt, cbrt);

pub fn min(args: &[Value]) -> Value {
    let n = args.iter()
        .filter_map(|v| extract_num(Some(v)))
        .fold(f64::INFINITY, f64::min);
    Value::Number(if n == f64::INFINITY { f64::NAN } else { n })
}

pub fn max(args: &[Value]) -> Value {
    let n = args.iter()
        .filter_map(|v| extract_num(Some(v)))
        .fold(f64::NEG_INFINITY, f64::max);
    Value::Number(if n == f64::NEG_INFINITY { f64::NAN } else { n })
}

pub fn pow(args: &[Value]) -> Value {
    let base = extract_num(args.first()).unwrap_or(f64::NAN);
    let exp = extract_num(args.get(1)).unwrap_or(f64::NAN);
    Value::Number(base.powf(exp))
}

pub fn random(_args: &[Value]) -> Value {
    Value::Number(rand::random::<f64>())
}

pub fn sign(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(if n.is_nan() {
        f64::NAN
    } else if n > 0.0 {
        1.0
    } else if n < 0.0 {
        -1.0
    } else {
        0.0
    })
}

pub fn atan2(args: &[Value]) -> Value {
    let y = extract_num(args.first()).unwrap_or(f64::NAN);
    let x = extract_num(args.get(1)).unwrap_or(f64::NAN);
    Value::Number(y.atan2(x))
}

pub fn hypot(args: &[Value]) -> Value {
    let x = extract_num(args.first()).unwrap_or(0.0);
    let y = extract_num(args.get(1)).unwrap_or(0.0);
    Value::Number(x.hypot(y))
}
