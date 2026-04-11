//! Bytecode opcodes for the Tish VM.

/// Stack-based bytecode opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    /// No operation
    Nop = 0,
    /// Push constant from constants table (operand: u16 index)
    LoadConst = 1,
    /// Load variable from scope (operand: u16 name index)
    LoadVar = 2,
    /// Store top of stack to variable (operand: u16 name index)
    StoreVar = 3,
    /// Discard top of stack
    Pop = 4,
    /// Duplicate top of stack
    Dup = 5,
    /// Call function with n args (operand: u16 arg count). Callee and args on stack.
    Call = 6,
    /// Return from function. Top of stack is return value.
    Return = 7,
    /// Unconditional jump forward (operand: u16 byte offset)
    Jump = 8,
    /// Pop top; if falsy, jump forward (operand: u16 byte offset)
    JumpIfFalse = 9,
    /// Unconditional jump backward (operand: u16 byte offset)
    JumpBack = 10,
    /// Binary operation (operand: u8 BinOp variant). Pops left, right; pushes result.
    BinOp = 11,
    /// Unary operation (operand: u8 UnaryOp variant). Pops operand; pushes result.
    UnaryOp = 12,
    /// Get property: obj.prop (operand: u16 prop name index). Pops obj; pushes value.
    GetMember = 13,
    /// Set property: obj.prop = val (operand: u16 prop name index). Pops obj, val.
    SetMember = 14,
    /// Get index: obj[idx]. Pops obj, idx; pushes value.
    GetIndex = 15,
    /// Set index: obj[idx] = val. Pops obj, idx, val.
    SetIndex = 16,
    /// Create array with n elements (operand: u16 count). Elements on stack.
    NewArray = 17,
    /// Create object with n key-value pairs (operand: u16 count). Keys and vals interleaved.
    NewObject = 18,
    /// Load from global scope (operand: u16 name index)
    LoadGlobal = 19,
    /// Store to global scope (operand: u16 name index)
    StoreGlobal = 20,
    /// Create closure: push function (operand: u16 chunk index for nested function)
    Closure = 21,
    /// Pop and discard n values (operand: u16 count)
    PopN = 22,
    /// Load `this` or receiver (for method calls)
    LoadThis = 23,
    /// Throw: pop value, unwind to catch handler, push value, jump
    Throw = 24,
    /// EnterTry: push handler (catch offset u16). Catch offset = bytes from end of this insn.
    EnterTry = 25,
    /// ExitTry: pop try handler
    ExitTry = 26,
    /// Concat arrays: pop right, pop left, push left.concat(right). For spread.
    ConcatArray = 27,
    /// Merge objects: pop right, pop left, push Object.assign({}, left, right). For object spread.
    MergeObject = 28,
    /// Call with spread: pop args array, pop callee, call callee(...args).
    CallSpread = 29,
    /// Get property optional: like GetMember but returns Null if obj is null or prop missing.
    GetMemberOptional = 30,
    /// Pop array, sort numerically in place (operand: u8 0=asc, 1=desc), push array.
    /// Fast path for arr.sort((a,b)=>a-b) / arr.sort((a,b)=>b-a).
    ArraySortNumeric = 31,
    /// Pop array, sort by numeric property (operands: u16 prop_name_const_idx, u16 0=asc/1=desc).
    /// Fast path for arr.sort((a,b)=>a.prop-b.prop).
    ArraySortByProperty = 32,
    /// arr.map(x => x) - identity, returns array clone.
    ArrayMapIdentity = 33,
    /// arr.map(x => x op const) or arr.map(x => const op x). Operands: u8 binop, u16 const_idx, u8 param_left (0=param on left e.g. x*2, 1=param on right e.g. 2*x).
    ArrayMapBinOp = 34,
    /// arr.filter(x => x op const) or arr.filter(x => const op x). Operands: u8 binop, u16 const_idx, u8 param_left. Keeps elements where result is truthy.
    ArrayFilterBinOp = 35,
    /// Load built-in module export. Operands: u16 spec_const_idx, u16 export_name_const_idx. Pushes Value.
    LoadNativeExport = 36,
    /// `new callee(...args)` (operand: u16 arg count). Stack: callee, then args — same as Call.
    Construct = 37,
    /// `new callee(...spread)` — stack: args array, then callee (same order as CallSpread).
    ConstructSpread = 38,
    /// Declare `let`/`const` in the current lexical frame (operand: u16 name index). Pops value.
    /// Does not walk enclosing scopes or globals (unlike [`StoreVar`]).
    DeclareVar = 39,
    /// Enter a block scope; pairs with [`ExitBlock`].
    EnterBlock = 40,
    /// Exit innermost block scope and restore shadowed bindings.
    ExitBlock = 41,
    /// Like [`DeclareVar`] but does not record block-scope undo (for `for`/`for-of` header bindings).
    DeclareVarPlain = 42,
}

impl Opcode {
    /// Decode byte to opcode. Safe for b in 0..=42 (matches #[repr(u8)] discriminants).
    #[inline]
    pub fn from_u8(b: u8) -> Option<Opcode> {
        if b <= 42 {
            Some(unsafe { std::mem::transmute(b) })
        } else {
            None
        }
    }

    /// Size in bytes of this instruction at `ip` (including operands). Returns None if truncated.
    pub fn instruction_size(self, code: &[u8], ip: usize) -> Option<usize> {
        let size = match self {
            Opcode::Nop
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::Return
            | Opcode::ExitTry
            | Opcode::ArrayMapIdentity
            | Opcode::CallSpread
            | Opcode::ConstructSpread
            | Opcode::EnterBlock
            | Opcode::ExitBlock => 1,
            Opcode::ArraySortByProperty
            | Opcode::ArrayMapBinOp
            | Opcode::ArrayFilterBinOp
            | Opcode::LoadNativeExport => 5,
            _ => 3,
        };
        if ip + size > code.len() {
            return None;
        }
        Some(size)
    }
}
