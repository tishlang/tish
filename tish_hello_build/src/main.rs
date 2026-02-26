use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tish_runtime::{print as tish_print, Value};

fn main() {
    let print = Value::Function(Rc::new(|args: &[Value]| {
        tish_print(args);
        Value::Null
    }));
    ({
        let f = &print;
        match f { Value::Function(cb) => cb(&[Value::String("Hello, Tish!".into())]), _ => panic!("Not a function") }
    });
    ({
        let f = &print;
        match f { Value::Function(cb) => cb(&[{ match (&Value::Number(1_f64), &Value::Number(2_f64)) {
                    (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
                    (Value::String(a), Value::String(b)) => Value::String(format!("{}{}", a, b).into()),
                    (a, b) => Value::String(format!("{}{}", a.to_display_string(), b.to_display_string()).into()),
                } }]), _ => panic!("Not a function") }
    });
    let mut x = Value::Number(3_f64);
    ({
        let f = &print;
        match f { Value::Function(cb) => cb(&[Value::Number({ let Value::Number(a) = &x else { panic!() }; let Value::Number(b) = &Value::Number(2_f64) else { panic!() }; a * b })]), _ => panic!("Not a function") }
    });
}
