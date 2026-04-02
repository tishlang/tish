//! Runtime linked into the `tish build --native-backend cranelift` executable.
//!
//! **`tish_run_chunk`** deserializes embedded bytecode and runs **`tishlang_vm`** — the same
//! execution engine as `tish run --backend vm`. The crate name is historical; this is not
//! running CLIF-emitted machine code for each Tish opcode.

use tishlang_bytecode::deserialize;
use tishlang_vm::Vm;

/// Serialization format:
/// - u64: code len
/// - bytes: code
/// - u64: constants count
/// - for each constant: u8 tag + payload
/// - u64: names count
/// - for each name: u64 len + bytes
///
/// Rust-callable wrapper. Run serialized chunk data. Returns exit code (0 on success).
pub fn tish_run_chunk(ptr: *const u8, len: usize) -> i32 {
    tish_run_chunk_impl(ptr, len)
}

#[no_mangle]
extern "C" fn tish_run_chunk_impl(ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() || len < 8 {
        return 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    match deserialize(slice) {
        Ok(chunk) => {
            let mut vm = Vm::new();
            match vm.run(&chunk) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("Runtime error: {}", e);
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("Deserialization error: {}", e);
            1
        }
    }
}
