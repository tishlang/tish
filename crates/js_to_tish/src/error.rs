//! Conversion error types.

use std::fmt;

/// Error returned when JS cannot be converted to Tish.
#[derive(Debug)]
pub struct ConvertError {
    pub kind: ConvertErrorKind,
}

impl ConvertError {
    pub fn new(kind: ConvertErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for ConvertError {}

/// Categorizes conversion failures.
#[derive(Debug)]
pub enum ConvertErrorKind {
    /// Parse error from OXC.
    Parse(String),
    /// Semantic analysis error from OXC.
    Semantic(String),
    /// Unsupported construct (class, this, for-in, etc.).
    Unsupported { what: String, hint: Option<String> },
    /// JS feature that cannot be expressed in Tish.
    Incompatible { what: String, reason: String },
}

impl fmt::Display for ConvertErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertErrorKind::Parse(s) => write!(f, "parse error: {s}"),
            ConvertErrorKind::Semantic(s) => write!(f, "semantic error: {s}"),
            ConvertErrorKind::Unsupported { what, hint } => {
                write!(f, "unsupported: {what}")?;
                if let Some(h) = hint {
                    write!(f, " ({h})")?;
                }
                Ok(())
            }
            ConvertErrorKind::Incompatible { what, reason } => {
                write!(f, "incompatible: {what} — {reason}")
            }
        }
    }
}
