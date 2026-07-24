//! ECMAScript-style `Symbol`, `Symbol.for`, `Symbol.keyFor`.

#[cfg(feature = "portable")]
#[allow(unused_imports)]
use alloc::{borrow::ToOwned, boxed::Box, format, string::{String, ToString}, vec, vec::Vec};

use tishlang_core::sync::{Mutex, OnceLock};
use tishlang_core::{alloc_symbol_id, AHashMap, Arc, ObjectMap, TishSymbol, Value};

static SYMBOL_FOR_REGISTRY: OnceLock<Mutex<AHashMap<Arc<str>, Arc<TishSymbol>>>> = OnceLock::new();

fn symbol_registry() -> &'static Mutex<AHashMap<Arc<str>, Arc<TishSymbol>>> {
    SYMBOL_FOR_REGISTRY.get_or_init(|| Mutex::new(AHashMap::default()))
}

fn symbol_for_impl(key: &str) -> Value {
    let k: Arc<str> = key.into();
    let mut reg = symbol_registry().lock().unwrap();
    // get-then-insert (rather than the `Entry` API) so this compiles against both
    // std `HashMap` and the portable `hashbrown` map without a hasher-specific
    // `Entry` import. `Symbol.for` is not a hot path.
    if let Some(existing) = reg.get(&k) {
        return Value::Symbol(existing.clone());
    }
    let id = alloc_symbol_id();
    let sym = TishSymbol::new_registry(id, Arc::clone(&k), None);
    reg.insert(k, Arc::clone(&sym));
    Value::Symbol(sym)
}

fn symbol_new(args: &[Value]) -> Value {
    let desc = args.first().and_then(|v| {
        if matches!(v, Value::Null) {
            None
        } else {
            Some(v.to_display_string().into())
        }
    });
    Value::Symbol(TishSymbol::new_unique(desc))
}

fn symbol_key_for_impl(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Symbol(s)) => s
            .registry_key
            .as_ref()
            .map(|k| Value::String(tishlang_core::ArcStr::from(k.as_ref())))
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Global `Symbol`: `Symbol("desc")` via `__call` / `__construct`, `Symbol.for`, `Symbol.keyFor`.
pub fn symbol_object() -> Value {
    let call = Value::native(symbol_new);
    let for_fn = Value::native(|args: &[Value]| {
        let key = args
            .first()
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        symbol_for_impl(&key)
    });
    let key_for = Value::native(symbol_key_for_impl);
    let mut m = ObjectMap::default();
    m.insert(Arc::from("__call"), call.clone());
    m.insert(Arc::from("__construct"), call);
    m.insert(Arc::from("for"), for_fn);
    m.insert(Arc::from("keyFor"), key_for);
    Value::object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tishlang_core::value_call;

    #[test]
    fn symbol_global_value_call() {
        let o = symbol_object();
        let r = value_call(&o, &[Value::String("hi".into())]);
        assert!(matches!(r, Value::Symbol(_)));
    }
}
