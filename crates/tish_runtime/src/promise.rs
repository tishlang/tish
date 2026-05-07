//! Promise helpers for the bytecode VM and native codegen (`Promise.resolve`, etc.).
//!
//! The global `Promise` value is an **object** with a `__call` entry so the VM can
//! invoke `Promise(executor)` like `new Promise(executor)` in JS. Static methods live
//! on the same object (`resolve`, `reject`, `all`, `race`).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tishlang_core::{ObjectMap, TishPromise, Value, VmRef};

/// Fulfilled or rejected before anyone awaits — `block_until_settled` consumes the result once.
pub struct ImmediateSettledPromise {
    slot: Mutex<Option<Result<Value, Value>>>,
}

impl ImmediateSettledPromise {
    fn new(result: Result<Value, Value>) -> Self {
        Self {
            slot: Mutex::new(Some(result)),
        }
    }
}

impl TishPromise for ImmediateSettledPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        self.slot
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(Value::String(
                "Promise already settled or consumed".into(),
            )))
    }
}

fn fulfilled(v: Value) -> Value {
    Value::Promise(Arc::new(ImmediateSettledPromise::new(Ok(v))))
}

fn rejected(v: Value) -> Value {
    Value::Promise(Arc::new(ImmediateSettledPromise::new(Err(v))))
}

fn flatten_chain_out(v: Value) -> std::result::Result<Value, Value> {
    match v {
        Value::Promise(p) => p.block_until_settled(),
        other => Ok(other),
    }
}

/// `Promise(executor)` — executor runs synchronously; `resolve` / `reject` unblock `recv`.
struct DeferredChannelPromise {
    rx: Mutex<Option<mpsc::Receiver<Result<Value, Value>>>>,
}

impl TishPromise for DeferredChannelPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        match rx {
            Some(r) => r.recv().unwrap_or(Err(Value::String(
                "Promise executor did not call resolve or reject".into(),
            ))),
            None => Err(Value::String(
                "Promise already consumed or settled".into(),
            )),
        }
    }
}

/// `.then` / `.catch` chain: when awaited, settle the predecessor then optionally invoke a handler.
pub struct ThenPromise {
    pred: Arc<dyn TishPromise>,
    on_fulfilled: Option<Value>,
    on_rejected: Option<Value>,
}

impl TishPromise for ThenPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        match self.pred.block_until_settled() {
            Ok(v) => {
                if let Some(Value::Function(f)) = &self.on_fulfilled {
                    flatten_chain_out(f(&[v]))
                } else {
                    Ok(v)
                }
            }
            Err(e) => {
                if let Some(Value::Function(f)) = &self.on_rejected {
                    flatten_chain_out(f(&[e]))
                } else {
                    Err(e)
                }
            }
        }
    }
}

/// `Promise.resolve(value)` — adopt promises, otherwise wrap in a fulfilled promise.
pub fn promise_resolve(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Promise(p)) => Value::Promise(Arc::clone(p)),
        Some(v) => fulfilled(v.clone()),
        None => fulfilled(Value::Null),
    }
}

/// `Promise.reject(reason)` — always a rejected promise.
pub fn promise_reject(args: &[Value]) -> Value {
    rejected(
        args.first()
            .cloned()
            .unwrap_or(Value::Null),
    )
}

/// `Promise.all(iterable)` — block on each promise in order; non-promises pass through.
pub fn promise_all(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Array(arr)) => {
            let mut out: Vec<Value> = Vec::new();
            for v in arr.borrow().iter() {
                let item = if let Value::Promise(p) = v {
                    match p.block_until_settled() {
                        Ok(x) => x,
                        Err(rej) => return rejected(rej),
                    }
                } else {
                    v.clone()
                };
                out.push(item);
            }
            fulfilled(Value::Array(VmRef::new(out)))
        }
        Some(v) => fulfilled(v.clone()),
        None => fulfilled(Value::Null),
    }
}

/// `Promise.race(iterable)` — first element wins (blocking first promise if it is one).
pub fn promise_race(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Array(arr)) => {
            let borrowed = arr.borrow();
            for v in borrowed.iter() {
                if let Value::Promise(p) = v {
                    return match p.block_until_settled() {
                        Ok(x) => fulfilled(x),
                        Err(e) => rejected(e),
                    };
                }
                return fulfilled(v.clone());
            }
            Value::Null
        }
        Some(v) => fulfilled(v.clone()),
        None => Value::Null,
    }
}

/// Build the global `Promise` object: `__call` (constructor) + static methods.
pub fn promise_object() -> Value {
    let mut map: ObjectMap = ObjectMap::default();

    let ctor = Value::native(|args: &[Value]| match args.first() {
        Some(Value::Function(f)) => {
            let (tx, rx) = mpsc::channel();
            let tx_cell = Arc::new(Mutex::new(Some(tx)));
            let resolve = Value::native({
                let tx_cell = Arc::clone(&tx_cell);
                move |a: &[Value]| {
                    if let Some(t) = tx_cell.lock().unwrap().take() {
                        let _ = t.send(Ok(
                            a.first().cloned().unwrap_or(Value::Null),
                        ));
                    }
                    Value::Null
                }
            });
            let reject = Value::native({
                let tx_cell = Arc::clone(&tx_cell);
                move |a: &[Value]| {
                    if let Some(t) = tx_cell.lock().unwrap().take() {
                        let _ = t.send(Err(
                            a.first().cloned().unwrap_or(Value::Null),
                        ));
                    }
                    Value::Null
                }
            });
            let _ = f(&[resolve, reject]);
            Value::Promise(Arc::new(DeferredChannelPromise {
                rx: Mutex::new(Some(rx)),
            }))
        }
        _ => Value::Null,
    });

    map.insert(Arc::from("__call"), ctor);
    map.insert(
        Arc::from("resolve"),
        Value::native(|args: &[Value]| promise_resolve(args)),
    );
    map.insert(
        Arc::from("reject"),
        Value::native(|args: &[Value]| promise_reject(args)),
    );
    map.insert(
        Arc::from("all"),
        Value::native(|args: &[Value]| promise_all(args)),
    );
    map.insert(
        Arc::from("race"),
        Value::native(|args: &[Value]| promise_race(args)),
    );
    Value::object(map)
}

/// `.then(onFulfilled, onRejected)` for a `Value::Promise` instance (VM `GetMember`).
pub fn promise_instance_then(p: &Arc<dyn TishPromise>, args: &[Value]) -> Value {
    Value::Promise(Arc::new(ThenPromise {
        pred: Arc::clone(p),
        on_fulfilled: args.first().cloned(),
        on_rejected: args.get(1).cloned(),
    }))
}

/// `.catch(onRejected)` for a `Value::Promise` instance.
pub fn promise_instance_catch(p: &Arc<dyn TishPromise>, args: &[Value]) -> Value {
    Value::Promise(Arc::new(ThenPromise {
        pred: Arc::clone(p),
        on_fulfilled: None,
        on_rejected: args.first().cloned(),
    }))
}
