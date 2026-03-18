//! Promises carrying only Send payloads (string results for text(), etc.).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tish_core::{Value, TishPromise};
use tokio::sync::oneshot;

fn error_value(msg: String) -> Value {
    let mut obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(2);
    obj.insert(Arc::from("error"), Value::String(msg.into()));
    obj.insert(Arc::from("ok"), Value::Bool(false));
    Value::Object(Rc::new(RefCell::new(obj)))
}

pub struct StringResultPromise {
    pub(crate) rx: Mutex<Option<oneshot::Receiver<Result<String, String>>>>,
}

impl TishPromise for StringResultPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            let result = crate::http::block_on_http(rx);
            match result {
                Ok(Ok(s)) => Ok(Value::String(s.into())),
                Ok(Err(e)) => Err(error_value(e)),
                Err(_) => Err(Value::String("Promise dropped".into())),
            }
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }
}

pub fn string_result_promise(rx: oneshot::Receiver<Result<String, String>>) -> Value {
    Value::Promise(Arc::new(StringResultPromise {
        rx: Mutex::new(Some(rx)),
    }))
}
