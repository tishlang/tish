//! Bytecode compiler for Tish.
//! Compiles AST to stack-based bytecode for VM execution.

mod chunk;
mod compiler;
mod opcode;
mod serialize;

pub use chunk::{Chunk, Constant};
pub use compiler::{compile, CompileError};
pub use opcode::Opcode;
pub use serialize::{deserialize, serialize};
