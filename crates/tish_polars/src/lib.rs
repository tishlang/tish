//! Polars bindings for Tish.
//!
//! Exposes Polars DataFrame and basic operations to Tish scripts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use polars::io::{SerReader, SerWriter};
use polars::prelude::*;
use tish_core::{NativeFn, TishOpaque, Value};
use tish_eval::{TishNativeModule, Value as EvalValue};

/// Wrapper around Polars DataFrame for Tish.
#[derive(Clone)]
pub struct TishDataFrame {
    pub inner: DataFrame,
}

impl TishDataFrame {
    pub fn new(df: DataFrame) -> Self {
        Self { inner: df }
    }

    fn select(&self, args: &[Value]) -> Value {
        let cols = match args.first() {
            Some(Value::Array(arr)) => {
                let names: Vec<String> = arr
                    .borrow()
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect();
                names
            }
            _ => return Value::Null,
        };
        match self.inner.select(cols.as_slice()) {
            Ok(df) => Value::Opaque(Arc::new(TishDataFrame::new(df))),
            Err(_) => Value::Null,
        }
    }

    fn to_json(&self, _args: &[Value]) -> Value {
        let mut buf = Vec::new();
        let mut df = self.inner.clone();
        match JsonWriter::new(&mut buf)
            .with_json_format(JsonFormat::Json)
            .finish(&mut df)
        {
            Ok(()) => match String::from_utf8(buf) {
                Ok(s) => Value::String(s.into()),
                Err(_) => Value::Null,
            },
            Err(_) => Value::Null,
        }
    }

    fn shape(&self, _args: &[Value]) -> Value {
        let (rows, cols) = self.inner.shape();
        let arr = vec![
            Value::Number(rows as f64),
            Value::Number(cols as f64),
        ];
        Value::Array(Rc::new(RefCell::new(arr)))
    }
}

impl TishOpaque for TishDataFrame {
    fn type_name(&self) -> &'static str {
        "DataFrame"
    }

    fn get_method(&self, name: &str) -> Option<NativeFn> {
        let inner = self.inner.clone();
        match name {
            "select" => Some(Rc::new(move |args: &[Value]| {
                let wrapper = TishDataFrame { inner: inner.clone() };
                wrapper.select(args)
            })),
            "toJson" | "to_json" => Some(Rc::new(move |args: &[Value]| {
                let wrapper = TishDataFrame { inner: inner.clone() };
                wrapper.to_json(args)
            })),
            "shape" => Some(Rc::new(move |args: &[Value]| {
                let wrapper = TishDataFrame { inner: inner.clone() };
                wrapper.shape(args)
            })),
            _ => None,
        }
    }
}

/// Runtime-compatible Polars.read_csv for compiled output (tish_core::Value).
pub fn polars_read_csv_runtime(args: &[Value]) -> Value {
    use std::path::PathBuf;
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    let path_buf: PathBuf = path.into();
    match CsvReadOptions::default().try_into_reader_with_file_path(Some(path_buf)) {
        Ok(reader) => match reader.finish() {
            Ok(df) => Value::Opaque(Arc::new(TishDataFrame::new(df))),
            Err(_) => Value::Null,
        },
        Err(_) => Value::Null,
    }
}

/// Read CSV from in-memory string (for compile-time embedded data).
pub fn polars_read_csv_from_string_runtime(csv_content: &str) -> Value {
    use std::io::Cursor;
    match CsvReader::new(Cursor::new(csv_content.as_bytes())).finish() {
        Ok(df) => Value::Opaque(Arc::new(TishDataFrame::new(df))),
        Err(_) => Value::Null,
    }
}

/// Native function for Polars.read_csv - must be a fn pointer for Value::Native (interpreter).
pub fn polars_read_csv(args: &[EvalValue]) -> Result<EvalValue, String> {
    use std::path::PathBuf;
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    let path_buf: PathBuf = path.into();
    match CsvReadOptions::default().try_into_reader_with_file_path(Some(path_buf)) {
        Ok(reader) => match reader.finish() {
            Ok(df) => Ok(EvalValue::Opaque(Arc::new(TishDataFrame::new(df)))),
            Err(e) => Err(e.to_string()),
        },
        Err(e) => Err(e.to_string()),
    }
}

/// Polars native module for Tish.
pub struct PolarsModule;

impl TishNativeModule for PolarsModule {
    fn name(&self) -> &'static str {
        "Polars"
    }

    fn register(&self) -> HashMap<Arc<str>, EvalValue> {
        let mut polars = HashMap::new();
        polars.insert(
            Arc::from("read_csv"),
            EvalValue::Native(polars_read_csv),
        );
        let mut scope = HashMap::new();
        scope.insert(
            Arc::from("Polars"),
            EvalValue::Object(Rc::new(RefCell::new(polars))),
        );
        scope
    }
}
