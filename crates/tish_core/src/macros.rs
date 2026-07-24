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
        use $crate::__TishArc as Arc;
        use $crate::{ObjectMap, Value};
        let mut map = ObjectMap::default();
        $(
            // `Value::native` picks the right Rc / Arc wrapper depending on
            // whether the `send-values` feature is enabled upstream.
            map.insert(Arc::from($name), Value::native($fn));
        )*
        Value::object(map)
    }};
}
