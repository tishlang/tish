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
