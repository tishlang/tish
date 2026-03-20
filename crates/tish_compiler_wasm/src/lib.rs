//! Tish compiler exposed to JS via wasm-bindgen.
//! Compiles single source string → bytecode (base64) or JS. Used by playground, REPL, try-it pages.

use base64::Engine;
use tish_compile_js::JsxMode;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn compile_to_bytecode(source: &str) -> Result<String, JsValue> {
    let program = tish_parser::parse(source.trim()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let program = tish_opt::optimize(&program);
    let chunk = tish_bytecode::compile(&program).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(tish_bytecode::serialize(&chunk)))
}

#[wasm_bindgen]
pub fn compile_to_js(source: &str) -> Result<String, JsValue> {
    let program = tish_parser::parse(source.trim()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    tish_compile_js::compile_with_jsx(&program, true, JsxMode::LegacyDom)
        .map_err(|e| JsValue::from_str(&e.message))
}
