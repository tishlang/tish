//! Bytecode chunk: instructions and constants.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tishlang_core::Value;

/// Per-property-name inline cache for object access (the JavaScriptCore inline-cache idea), indexed by
/// the name index that `GetMember`/`SetMember` already carry. Each cell packs
/// `(shape_id:u32 << 32) | slot_index:u32`; `0` = uncached. A racy `Relaxed` load/store is sound: a
/// stale read just falls to the slow path, which re-checks the object's shape and refills. This is a
/// runtime cache, NOT program data — a cloned `Chunk` (e.g. each closure instance) starts empty.
#[derive(Debug, Default)]
pub struct InlineCaches(pub Vec<AtomicU64>);

impl Clone for InlineCaches {
    fn clone(&self) -> Self {
        InlineCaches(self.0.iter().map(|_| AtomicU64::new(0)).collect())
    }
}

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
            Constant::String(s) => Value::String(tishlang_core::ArcStr::from(s.as_ref())),
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
    /// Inline caches for object property access, one cell per entry in `names` (so indexed by the
    /// same name index `GetMember`/`SetMember` carry). Runtime-only; not part of the serialized program.
    pub inline_caches: InlineCaches,
    /// Source line table: `(code_offset, line)` pairs, sorted by offset, one entry per line change
    /// (issue #74). Consulted only when formatting a runtime error, so it adds zero execution
    /// overhead. Debug-only / runtime-only — not serialized (persisted bytecode loses line info).
    pub lines: Vec<(u32, u32)>,
    /// Source file path for error messages (`file:line`); propagated to nested chunks. Runtime-only.
    pub source: Option<Arc<str>>,
    /// #187: when `Some(name)`, this chunk is a top-level `function name` whose binding is provably
    /// stable across the whole program (never reassigned/shadowed/redeclared). The numeric JIT
    /// registers such a chunk under `name` so a caller's `name(args)` can lower to a direct native
    /// call. `None` for anonymous/nested/unstable functions. Runtime-only; not serialized (a reloaded
    /// program just forgoes the cross-function-call optimization).
    pub global_name: Option<Arc<str>>,
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
            inline_caches: InlineCaches::default(),
            lines: Vec::new(),
            source: None,
            global_name: None,
        }
    }

    /// Record that the instruction starting at `offset` originates from source `line` (1-based).
    /// Only stored when the line changes, keeping the table compact. Issue #74.
    pub fn mark_line(&mut self, offset: usize, line: u32) {
        if line == 0 {
            return;
        }
        match self.lines.last() {
            Some(&(_, last_line)) if last_line == line => {}
            _ => self.lines.push((offset as u32, line)),
        }
    }

    /// Source line for a bytecode `offset` (the line of the nearest preceding `mark_line`), or
    /// `None` if no line info is available (e.g. deserialized bytecode). Issue #74.
    pub fn line_at(&self, offset: usize) -> Option<u32> {
        if self.lines.is_empty() {
            return None;
        }
        let off = offset as u32;
        // Largest recorded offset <= `off`.
        let idx = self.lines.partition_point(|&(o, _)| o <= off);
        if idx == 0 {
            Some(self.lines[0].1)
        } else {
            Some(self.lines[idx - 1].1)
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
        self.inline_caches.0.push(AtomicU64::new(0)); // keep the IC table sized to `names`
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
