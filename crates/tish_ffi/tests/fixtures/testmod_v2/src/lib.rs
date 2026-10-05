use tishlang_core::{take_pending_throw, Value, VmRef};
use tishlang_ffi_guest as guest;
use guest::{TishExportTable, TishHostApi, TishValueRef};

fn no_throw() -> Value {
    Value::Null
}

fn parked_throw() -> Value {
    take_pending_throw().unwrap_or(Value::Null)
}

/// `describe(obj)` → "k1,k2|<count>": reads a host object's keys (v1 could not).
extern "C" fn describe(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        guest::export_call(
            args,
            argc,
            |a| match a.first() {
                Some(Value::Object(o)) => {
                    let o = o.borrow();
                    let keys: Vec<String> = o.strings.keys().map(|k| k.to_string()).collect();
                    Value::String(format!("{}|{}", keys.join(","), keys.len()).into())
                }
                _ => Value::Null,
            },
            no_throw,
        )
    }
}

/// `apply(f, x)` → `f(x) + 1`: calls a host function from the module.
extern "C" fn apply(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        guest::export_call(
            args,
            argc,
            |a| match (a.first(), a.get(1)) {
                (Some(Value::Function(f)), Some(x)) => match f.call(&[x.clone()]) {
                    Value::Number(n) => Value::Number(n + 1.0),
                    other => other,
                },
                _ => Value::Null,
            },
            parked_throw,
        )
    }
}

/// `make_adder(n)` → a module function `(x) => x + n` the host can call later.
extern "C" fn make_adder(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        guest::export_call(
            args,
            argc,
            |a| {
                let n = match a.first() {
                    Some(Value::Number(n)) => *n,
                    _ => 0.0,
                };
                Value::native(move |xs: &[Value]| match xs.first() {
                    Some(Value::Number(x)) => Value::Number(x + n),
                    _ => Value::Null,
                })
            },
            no_throw,
        )
    }
}

/// `boom(msg)` throws `msg` into the host.
extern "C" fn boom(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        let msg = std::cell::RefCell::new(Value::Null);
        guest::export_call(
            args,
            argc,
            |a| {
                *msg.borrow_mut() = a.first().cloned().unwrap_or(Value::Null);
                Value::Null
            },
            || msg.borrow().clone(),
        )
    }
}

/// `echo(x)` returns a deep copy of `x` (arrays, nested objects, strings with NUL).
extern "C" fn echo(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        guest::export_call(
            args,
            argc,
            |a| a.first().cloned().unwrap_or(Value::Array(VmRef::new(Vec::new()))),
            no_throw,
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn tish_module_register_v2(api: *const TishHostApi) -> *const TishExportTable {
    if !guest::install(api) {
        return std::ptr::null();
    }
    guest::export_table(&[
        (b"describe\0", describe),
        (b"apply\0", apply),
        (b"make_adder\0", make_adder),
        (b"boom\0", boom),
        (b"echo\0", echo),
    ])
}
