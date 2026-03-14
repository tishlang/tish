//! Bytecode compiler for Tish.
//! Compiles AST to stack-based bytecode for VM execution.

mod chunk;
mod compiler;
mod opcode;
mod peephole;
mod serialize;

pub const NO_REST_PARAM: u16 = 0xFFFF;

pub use chunk::{Chunk, Constant};
pub use compiler::{compile, compile_unoptimized, CompileError};
pub use opcode::Opcode;
pub use serialize::{deserialize, serialize};
