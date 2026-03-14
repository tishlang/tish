//! Bytecode compiler for Tish.
//! Compiles AST to stack-based bytecode for VM execution.

mod chunk;
mod compiler;
mod encoding;
mod opcode;
mod peephole;
mod serialize;

pub const NO_REST_PARAM: u16 = 0xFFFF;

pub use chunk::{Chunk, Constant};
pub use compiler::{compile, compile_unoptimized, CompileError};
pub use encoding::{binop_to_u8, compound_op_to_u8, u8_to_binop, u8_to_unaryop, unaryop_to_u8};
pub use opcode::Opcode;
pub use serialize::{deserialize, serialize};
