//! Promises carrying only Send payloads (string results for text(), etc.).

use std::sync::{Arc, Mutex};
use tishlang_core::{ObjectMap, TishPromise, Value};
use tokio::sync::oneshot;

fn error_value(msg: String) -> Value {
    let mut obj: ObjectMap = ObjectMap::with_capacity(2);
    obj.insert(Arc::from("error"), Value::String(msg.into()));
    obj.insert(Arc::from("ok"), Value::Bool(false));
    Value::object(obj)
}

pub struct StringResultPromise {
    pub(crate) rx: Mutex<Option<oneshot::Receiver<Result<String, String>>>>,
}

impl StringResultPromise {
    fn convert(
        r: Result<Result<String, String>, oneshot::error::RecvError>,
    ) -> std::result::Result<Value, Value> {
        match r {
            Ok(Ok(s)) => Ok(Value::String(s.into())),
            Ok(Err(e)) => Err(error_value(e)),
            Err(_) => Err(Value::String("Promise dropped".into())),
        }
    }
}

impl TishPromise for StringResultPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            Self::convert(crate::http::block_on_http(rx))
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }

    /// Await the oneshot on the shared subscriber runtime instead of parking a
    /// blocking thread (plus a per-call tokio runtime) per raced input (issue #702).
    fn subscribe(
        self: Arc<Self>,
        on_settled: Box<dyn FnOnce(std::result::Result<Value, Value>) + Send>,
    ) {
        match self.rx.lock().unwrap().take() {
            Some(rx) => {
                crate::http::subscriber_runtime().spawn(async move {
                    on_settled(Self::convert(rx.await));
                });
            }
            None => on_settled(Err(Value::String("Promise already consumed".into()))),
        }
    }
}

pub fn string_result_promise(rx: oneshot::Receiver<Result<String, String>>) -> Value {
    Value::Promise(Arc::new(StringResultPromise {
        rx: Mutex::new(Some(rx)),
    }))
}
