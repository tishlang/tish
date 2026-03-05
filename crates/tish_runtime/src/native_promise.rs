//! Native Promise for compiled Tish - allows fetchAsync to return a Promise
//! that can be passed to Promise.all and awaited.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tish_core::{Value, TishPromise};
use tokio::sync::oneshot;

fn fetch_result_to_value(r: super::http::FetchResultPrimitive) -> Value {
    let headers_obj: HashMap<Arc<str>, Value> = r
        .headers
        .into_iter()
        .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
        .collect();
    let mut obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(4);
    obj.insert(Arc::from("status"), Value::Number(r.status));
    obj.insert(Arc::from("ok"), Value::Bool(r.ok));
    obj.insert(Arc::from("body"), Value::String(r.body.into()));
    obj.insert(
        Arc::from("headers"),
        Value::Object(Rc::new(RefCell::new(headers_obj))),
    );
    Value::Object(Rc::new(RefCell::new(obj)))
}

fn error_value(msg: String) -> Value {
    let mut obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(2);
    obj.insert(Arc::from("error"), Value::String(msg.into()));
    obj.insert(Arc::from("ok"), Value::Bool(false));
    Value::Object(Rc::new(RefCell::new(obj)))
}

/// Promise that holds a oneshot receiver - result is sent from a spawned task.
struct NativePromise {
    rx: Mutex<Option<oneshot::Receiver<Result<super::http::FetchResultPrimitive, String>>>>,
}

impl TishPromise for NativePromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(rx)
            });
            match result {
                Ok(Ok(fetch_result)) => Ok(fetch_result_to_value(fetch_result)),
                Ok(Err(e)) => Err(error_value(e)),
                Err(_) => Err(Value::String("Promise dropped".into())),
            }
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }
}

/// Create a Promise that will resolve with the result of fetching the given URL.
/// Spawns the fetch on the runtime; returns immediately with the Promise.
pub fn fetch_async_promise(args: Vec<Value>) -> Value {
    let url = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => return error_value("fetchAsync requires a URL".to_string()),
    };
    let options = args.get(1).cloned();
    let method = super::http::extract_method_from_value(options.as_ref());
    let headers = super::http::extract_headers_from_value(options.as_ref());
    let body = super::http::extract_body_from_value(options.as_ref());

    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let result = super::http::fetch_one_async_primitive(&url, &method, &headers, body).await;
        let _ = tx.send(result);
    });

    Value::Promise(Arc::new(NativePromise {
        rx: Mutex::new(Some(rx)),
    }))
}

/// If v is a Promise, block until settled and return the value. Otherwise return v as-is.
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
