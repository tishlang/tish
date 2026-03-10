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
}

impl Opcode {
    pub fn from_u8(b: u8) -> Option<Opcode> {
        match b {
            0 => Some(Opcode::Nop),
            1 => Some(Opcode::LoadConst),
            2 => Some(Opcode::LoadVar),
            3 => Some(Opcode::StoreVar),
            4 => Some(Opcode::Pop),
            5 => Some(Opcode::Dup),
            6 => Some(Opcode::Call),
            7 => Some(Opcode::Return),
            8 => Some(Opcode::Jump),
            9 => Some(Opcode::JumpIfFalse),
            10 => Some(Opcode::JumpBack),
            11 => Some(Opcode::BinOp),
            12 => Some(Opcode::UnaryOp),
            13 => Some(Opcode::GetMember),
            14 => Some(Opcode::SetMember),
            15 => Some(Opcode::GetIndex),
            16 => Some(Opcode::SetIndex),
            17 => Some(Opcode::NewArray),
            18 => Some(Opcode::NewObject),
            19 => Some(Opcode::LoadGlobal),
            20 => Some(Opcode::StoreGlobal),
            21 => Some(Opcode::Closure),
            22 => Some(Opcode::PopN),
            23 => Some(Opcode::LoadThis),
            _ => None,
        }
    }
}
