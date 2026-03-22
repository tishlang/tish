//! Macros for building Tish native modules.

/// Build a Tish module object from method name => function pairs.
///
/// Each function must have signature `fn(&[Value]) -> Value` (or equivalent closure).
/// Pass either a `fn` pointer or a closure; the macro wraps them in `Rc::new`.
///
/// # Example
///
/// ```ignore
/// use tishlang_core::{tish_module, Value};
///
/// pub fn my_object() -> Value {
///     tish_module! {
///         "run" => |args: &[Value]| {
///             // ...
///             Value::Null
///         },
///         "read_csv" => my_read_csv_fn,
///     }
/// }
/// ```
#[macro_export]
macro_rules! tish_module {
    ($($name:expr => $fn:expr),* $(,)?) => {{
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::rc::Rc;
        use std::sync::Arc;
        use $crate::Value;
        let mut map = HashMap::new();
        $(
            map.insert(Arc::from($name), Value::Function(Rc::new($fn)));
        )*
        Value::Object(Rc::new(RefCell::new(map)))
    }};
}
