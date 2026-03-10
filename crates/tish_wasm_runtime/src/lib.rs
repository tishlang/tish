//! Tish VM as a WebAssembly module for browser execution.
//!
//! Compile with: cargo build -p tish_wasm_runtime --target wasm32-unknown-unknown --release
//!
//! The resulting .wasm binary is the VM. Pass serialized bytecode from JavaScript.

use wasm_bindgen::prelude::*;

use tish_bytecode::deserialize;
use tish_vm::Vm;

/// Run serialized Tish bytecode in the browser.
///
/// `chunk` is the output of `tish_bytecode::serialize(chunk)`.
/// Errors are returned as a JsValue (string).
#[wasm_bindgen]
pub fn run(chunk: Vec<u8>) -> Result<(), JsValue> {
    let chunk = deserialize(&chunk).map_err(|e| JsValue::from_str(&e))?;
    let mut vm = Vm::new();
    vm.run(&chunk).map_err(|e| JsValue::from_str(&e))?;
    Ok(())
}
