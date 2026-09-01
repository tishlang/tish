//! ECMAScript-style `Symbol`, `Symbol.for`, `Symbol.keyFor`.

#[cfg(feature = "portable")]
#[allow(unused_imports)]
use alloc::{
    borrow::ToOwned,
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

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
            // ECMAScript ToString of the description (`Symbol({})` → "[object Object]"), not the
            // console-inspect form (#715). Conformance-only here: unique symbols are refcounted.
            Some(v.to_js_string().into())
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
        // The registry key is the ECMAScript ToString of the argument (spec: `Symbol.for`
        // coerces with ToString), NOT the console-inspect form (#715): every object keys the
        // single "[object Object]" entry and an array keys its comma-joined form, exactly as
        // in JS. The inspect form turned the immortal registry's program-text-bounded key set
        // into a data-driven one — `Symbol.for({id: n})` interned one permanent entry per
        // distinct rendered object. A missing argument keys "undefined" (ToString(undefined)).
        let key = args
            .first()
            .map(|v| v.to_js_string())
            .unwrap_or_else(|| "undefined".to_string());
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

    fn member(o: &Value, name: &str) -> Value {
        match o {
            Value::Object(obj) => obj
                .borrow()
                .strings
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("missing member {name}")),
            other => panic!("expected object, got {other:?}"),
        }
    }

    fn as_symbol(v: Value) -> Arc<TishSymbol> {
        match v {
            Value::Symbol(s) => s,
            other => panic!("expected symbol, got {other:?}"),
        }
    }

    fn test_object(n: f64) -> Value {
        let mut m = ObjectMap::default();
        m.insert(Arc::from("id"), Value::Number(n));
        Value::object(m)
    }

    // #715: the registry keys on ToString, so two DISTINCT objects intern the SAME entry
    // ("[object Object]") — previously each distinct inspect string ("{id: 1}", "{id: 2}", …)
    // interned one immortal registry entry per call.
    #[test]
    fn symbol_for_keys_objects_by_tostring() {
        let for_fn = member(&symbol_object(), "for");
        let a = as_symbol(value_call(&for_fn, &[test_object(1.0)]));
        let b = as_symbol(value_call(&for_fn, &[test_object(2.0)]));
        assert!(
            Arc::ptr_eq(&a, &b),
            "distinct objects must intern one shared '[object Object]' registry entry"
        );
        assert_eq!(a.registry_key.as_deref(), Some("[object Object]"));
    }

    // String and number args keep their (spec-shaped) keys; ToString makes Symbol.for(1) and
    // Symbol.for("1") the same symbol, as in JS.
    #[test]
    fn symbol_for_string_and_number_keys_unchanged() {
        let for_fn = member(&symbol_object(), "for");
        let s1 = as_symbol(value_call(&for_fn, &[Value::String("sym.key.715".into())]));
        let s2 = as_symbol(value_call(&for_fn, &[Value::String("sym.key.715".into())]));
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(s1.registry_key.as_deref(), Some("sym.key.715"));

        let n = as_symbol(value_call(&for_fn, &[Value::Number(1.0)]));
        let n_str = as_symbol(value_call(&for_fn, &[Value::String("1".into())]));
        assert_eq!(n.registry_key.as_deref(), Some("1"));
        assert!(
            Arc::ptr_eq(&n, &n_str),
            "Symbol.for(1) === Symbol.for(\"1\")"
        );
    }

    // A missing argument keys "undefined" (ToString(undefined)), not "".
    #[test]
    fn symbol_for_missing_arg_keys_undefined() {
        let for_fn = member(&symbol_object(), "for");
        let s = as_symbol(value_call(&for_fn, &[]));
        assert_eq!(s.registry_key.as_deref(), Some("undefined"));
    }
}
