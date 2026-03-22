//! Native Promise entrypoints for fetch / fetchAll.

use tishlang_core::Value;

pub fn fetch_promise(args: Vec<Value>) -> Value {
    crate::http_fetch::fetch_promise_from_args(args)
}

pub fn fetch_all_promise(args: Vec<Value>) -> Value {
    crate::http_fetch::fetch_all_promise_from_args(args)
}

pub fn fetch_async_promise(args: Vec<Value>) -> Value {
    fetch_promise(args)
}

pub fn await_promise(v: Value) -> Value {
    if let Value::Promise(p) = v {
        match p.block_until_settled() {
            Ok(val) => val,
            Err(rejection) => rejection,
        }
    } else {
        v
    }
}
