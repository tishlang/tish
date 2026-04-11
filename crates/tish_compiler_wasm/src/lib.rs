//! Tish compiler exposed to JS via wasm-bindgen.
//! Compiles single source string → bytecode (base64) or JS. Used by playground, REPL, try-it pages.
//!
//! `compile_to_js` / `compile_to_js_with_imports` use Lattish-style JSX lowering. Prepend a compiled
//! **Lattish.tish** runtime (same as playground `lattish-runtime.js`) so the iframe/script scope has
//! hooks and the JSX helpers; source files do not need to import the JSX helper by name.

mod resolve_virtual;

use base64::Engine;
use resolve_virtual::{detect_cycles_virtual, merge_modules_virtual, resolve_virtual};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn compile_to_bytecode(source: &str) -> Result<String, JsValue> {
    let program =
        tishlang_parser::parse(source.trim()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let program = tishlang_opt::optimize(&program);
    let chunk =
        tishlang_bytecode::compile(&program).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(tishlang_bytecode::serialize(&chunk)))
}

#[wasm_bindgen]
pub fn compile_to_js(source: &str) -> Result<String, JsValue> {
    let program =
        tishlang_parser::parse(source.trim()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    tishlang_compile_js::compile_with_jsx(&program, true).map_err(|e| JsValue::from_str(&e.message))
}

#[wasm_bindgen]
pub fn compile_to_bytecode_with_imports(
    entry_path: &str,
    files_json: &str,
) -> Result<String, JsValue> {
    let files: HashMap<String, String> = serde_json::from_str(files_json)
        .map_err(|e| JsValue::from_str(&format!("Invalid files JSON: {}", e)))?;
    let modules = resolve_virtual(entry_path, &files).map_err(|e| JsValue::from_str(&e))?;
    detect_cycles_virtual(&modules).map_err(|e| JsValue::from_str(&e))?;
    let program = merge_modules_virtual(modules).map_err(|e| JsValue::from_str(&e))?;
    let program = tishlang_opt::optimize(&program);
    let chunk =
        tishlang_bytecode::compile(&program).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(tishlang_bytecode::serialize(&chunk)))
}

#[wasm_bindgen]
pub fn compile_to_js_with_imports(entry_path: &str, files_json: &str) -> Result<String, JsValue> {
    let files: HashMap<String, String> = serde_json::from_str(files_json)
        .map_err(|e| JsValue::from_str(&format!("Invalid files JSON: {}", e)))?;
    let modules = resolve_virtual(entry_path, &files).map_err(|e| JsValue::from_str(&e))?;
    detect_cycles_virtual(&modules).map_err(|e| JsValue::from_str(&e))?;
    let program = merge_modules_virtual(modules).map_err(|e| JsValue::from_str(&e))?;
    let program = tishlang_opt::optimize(&program);
    tishlang_compile_js::compile_with_jsx(&program, true).map_err(|e| JsValue::from_str(&e.message))
}
