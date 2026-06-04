//! Bytecode chunk: instructions and constants.

use std::sync::Arc;
use tishlang_core::Value;

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
    /// Number of local variable slots this chunk's call frame needs (params + body locals).
    /// Frame `locals` Vec is sized to this. Only meaningful when `slot_based`.
    pub num_slots: u16,
    /// When true, this chunk resolves its locals via integer frame slots
    /// (`LoadLocal`/`StoreLocal`) instead of name-keyed scope maps. Set for
    /// self-contained functions (no free-variable / global references), whose
    /// call frame is a bare `Vec<Value>` of length `num_slots` — no per-call
    /// hashmap, no name lookups. Name-based chunks (top level, closures that
    /// capture outer scope) leave this `false` and use the legacy path.
    pub slot_based: bool,
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
            num_slots: 0,
            slot_based: false,
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
