//! Bytecode chunk: instructions and constants.

use std::sync::Arc;
use tish_core::Value;

/// A constant in the constants table.
#[derive(Debug, Clone)]
pub enum Constant {
    /// Primitive literals
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    /// Nested function code (index into parent Chunk's nested chunks)
    Closure(usize),
}

impl Constant {
    pub fn to_value(&self) -> Value {
        match self {
            Constant::Number(n) => Value::Number(*n),
            Constant::String(s) => Value::String(Arc::clone(s)),
            Constant::Bool(b) => Value::Bool(*b),
            Constant::Null => Value::Null,
            Constant::Closure(_) => {
                // Closures are converted to Value at runtime by the VM
                unreachable!("Closure constant should be handled by VM")
            }
        }
    }
}


/// A bytecode chunk: instructions and associated data.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Raw bytecode. Instructions are variable-length.
    pub code: Vec<u8>,
    /// Constants (literals, nested function indices)
    pub constants: Vec<Constant>,
    /// Variable/property names (strings). First `param_count` are parameter names.
    pub names: Vec<Arc<str>>,
    /// Nested chunks (for function bodies)
    pub nested: Vec<Chunk>,
    /// Index into names for rest param, or NO_REST_PARAM if none.
    pub rest_param_index: u16,
    /// Number of leading names that are parameters (for proper closure arg binding).
    pub param_count: u16,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
            nested: Vec::new(),
            rest_param_index: super::NO_REST_PARAM,
            param_count: 0,
        }
    }

    pub fn write_u8(&mut self, b: u8) {
        self.code.push(b);
    }

    pub fn write_u16(&mut self, n: u16) {
        self.code.extend_from_slice(&n.to_be_bytes());
    }

    pub fn add_constant(&mut self, c: Constant) -> u16 {
        let idx = self.constants.len();
        self.constants.push(c);
        idx as u16
    }

    pub fn add_name(&mut self, name: Arc<str>) -> u16 {
        if let Some(idx) = self.names.iter().position(|n| n.as_ref() == name.as_ref()) {
            return idx as u16;
        }
        let idx = self.names.len();
        self.names.push(name);
        idx as u16
    }

    pub fn add_nested(&mut self, chunk: Chunk) -> usize {
        let idx = self.nested.len();
        self.nested.push(chunk);
        idx
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}
