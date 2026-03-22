//! Canonical u8 encoding for AST operators in bytecode.
//! Single source of truth: compiler encodes with *\_to_u8, VM decodes with u8_to\_*.

use tishlang_ast::{BinOp, CompoundOp, UnaryOp};

/// Encode BinOp for bytecode operand. Used by compiler.
pub fn binop_to_u8(op: BinOp) -> u8 {
    use tishlang_ast::BinOp::*;
    match op {
        Add => 0,
        Sub => 1,
        Mul => 2,
        Div => 3,
        Mod => 4,
        Pow => 5,
        Eq => 6,
        Ne => 7,
        StrictEq => 8,
        StrictNe => 9,
        Lt => 10,
        Le => 11,
        Gt => 12,
        Ge => 13,
        And => 14,
        Or => 15,
        BitAnd => 16,
        BitOr => 17,
        BitXor => 18,
        Shl => 19,
        Shr => 20,
        In => 21,
    }
}

/// Decode bytecode operand to BinOp. Used by VM.
pub fn u8_to_binop(b: u8) -> Option<BinOp> {
    use tishlang_ast::BinOp::*;
    Some(match b {
        0 => Add,
        1 => Sub,
        2 => Mul,
        3 => Div,
        4 => Mod,
        5 => Pow,
        6 => Eq,
        7 => Ne,
        8 => StrictEq,
        9 => StrictNe,
        10 => Lt,
        11 => Le,
        12 => Gt,
        13 => Ge,
        14 => And,
        15 => Or,
        16 => BitAnd,
        17 => BitOr,
        18 => BitXor,
        19 => Shl,
        20 => Shr,
        21 => In,
        _ => return None,
    })
}

/// Encode CompoundOp for bytecode (same numeric subset as BinOp: Add,Sub,Mul,Div,Mod).
pub fn compound_op_to_u8(op: CompoundOp) -> u8 {
    use tishlang_ast::CompoundOp::*;
    match op {
        Add => 0,
        Sub => 1,
        Mul => 2,
        Div => 3,
        Mod => 4,
    }
}

/// Encode UnaryOp for bytecode operand. Used by compiler.
pub fn unaryop_to_u8(op: UnaryOp) -> u8 {
    use tishlang_ast::UnaryOp::*;
    match op {
        Not => 0,
        Neg => 1,
        Pos => 2,
        BitNot => 3,
        Void => 4,
    }
}

/// Decode bytecode operand to UnaryOp. Used by VM.
pub fn u8_to_unaryop(b: u8) -> Option<UnaryOp> {
    use tishlang_ast::UnaryOp::*;
    Some(match b {
        0 => Not,
        1 => Neg,
        2 => Pos,
        3 => BitNot,
        4 => Void,
        _ => return None,
    })
}
