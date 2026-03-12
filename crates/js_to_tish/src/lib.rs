//! Vanilla JavaScript to Tish AST converter.
//!
//! Parses JavaScript with OXC, runs semantic analysis for scope/hoisting,
//! normalizes to Tish's simpler model, and emits Tish AST.

mod error;
mod span_util;
mod transform;

pub use error::{ConvertError, ConvertErrorKind};
pub use transform::convert;
