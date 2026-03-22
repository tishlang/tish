//! JS-only call sites that require `new` (Tish has no `new`).
//! Intrinsic names, validation, and runtime preamble live here — main codegen only dispatches.

use tishlang_ast::{CallArg, Expr};

use crate::error::CompileError;

/// Built-in calls lowered to `new ...` in the JS emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsIntrinsic {
    WebAudioCreateContext,
    Uint8Array,
}

#[derive(Debug, Default)]
pub struct JsIntrinsics {
    pub needs_web_audio: bool,
    pub needs_uint8_array: bool,
}

impl JsIntrinsics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recognize `webAudioCreateContext()` / `jsUint8Array(n)` and validate arguments.
    pub fn classify_call(callee: &Expr, args: &[CallArg]) -> Result<Option<JsIntrinsic>, CompileError> {
        let Expr::Ident { name, .. } = callee else {
            return Ok(None);
        };
        match name.as_ref() {
            "webAudioCreateContext" => {
                if !args.is_empty() {
                    return Err(CompileError::new(
                        "webAudioCreateContext() takes no arguments (JS target only)",
                    ));
                }
                Ok(Some(JsIntrinsic::WebAudioCreateContext))
            }
            "jsUint8Array" => {
                if args.len() != 1 {
                    return Err(CompileError::new(
                        "jsUint8Array(length) expects one argument (JS target only)",
                    ));
                }
                Ok(Some(JsIntrinsic::Uint8Array))
            }
            _ => Ok(None),
        }
    }

    pub fn mark(&mut self, kind: JsIntrinsic) {
        match kind {
            JsIntrinsic::WebAudioCreateContext => self.needs_web_audio = true,
            JsIntrinsic::Uint8Array => self.needs_uint8_array = true,
        }
    }

    pub fn emit_expr(kind: JsIntrinsic, uint8_length_js: &str) -> String {
        match kind {
            JsIntrinsic::WebAudioCreateContext => "(__tishWebAudioCreateContext() ?? null)".to_string(),
            JsIntrinsic::Uint8Array => format!("(__tishUint8Array({}) ?? null)", uint8_length_js),
        }
    }

    /// Prepend runtime helpers (Uint8Array above AudioContext when both are used).
    pub fn prepend_runtime_preamble(&self, mut output: String) -> String {
        if self.needs_web_audio {
            output = format!(
                "function __tishWebAudioCreateContext(){{ return new AudioContext(); }}\n{}",
                output
            );
        }
        if self.needs_uint8_array {
            output = format!(
                "function __tishUint8Array(n){{ return new Uint8Array(n); }}\n{}",
                output
            );
        }
        output
    }
}
