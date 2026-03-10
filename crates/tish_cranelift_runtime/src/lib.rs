//! Runtime for Cranelift-compiled Tish programs.
//!
//! Provides tish_run_chunk(ptr, len) which deserializes and runs bytecode.

use std::sync::Arc;

use tish_bytecode::{Chunk, Constant};
use tish_vm::Vm;

/// Serialization format:
/// - u64: code len
/// - bytes: code
/// - u64: constants count
/// - for each constant: u8 tag + payload
/// - u64: names count
/// - for each name: u64 len + bytes
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
    match deserialize_chunk(slice) {
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

fn deserialize_chunk(mut data: &[u8]) -> Result<Chunk, String> {
    let read_u64 = |d: &mut &[u8]| {
        if d.len() < 8 {
            return Err("Unexpected EOF".to_string());
        }
        let (head, tail) = d.split_at(8);
        *d = tail;
        Ok(u64::from_le_bytes(head.try_into().unwrap()))
    };

    let code_len = read_u64(&mut data)? as usize;
    if data.len() < code_len {
        return Err("Truncated code".to_string());
    }
    let (code_bytes, rest) = data.split_at(code_len);
    data = rest;
    let code = code_bytes.to_vec();

    let const_count = read_u64(&mut data)? as usize;
    let mut constants = Vec::with_capacity(const_count);
    for _ in 0..const_count {
        if data.is_empty() {
            return Err("Truncated constant".to_string());
        }
        let tag = data[0];
        data = &data[1..];
        let c = match tag {
            0 => {
                if data.len() < 8 {
                    return Err("Truncated number".to_string());
                }
                let (n_bytes, rest) = data.split_at(8);
                data = rest;
                Constant::Number(f64::from_le_bytes(n_bytes.try_into().unwrap()))
            }
            1 => {
                let str_len = read_u64(&mut data)? as usize;
                if data.len() < str_len {
                    return Err("Truncated string".to_string());
                }
                let (s_bytes, rest) = data.split_at(str_len);
                data = rest;
                Constant::String(Arc::from(String::from_utf8_lossy(s_bytes).into_owned()))
            }
            2 => {
                if data.is_empty() {
                    return Err("Truncated bool".to_string());
                }
                let b = data[0] != 0;
                data = &data[1..];
                Constant::Bool(b)
            }
            3 => Constant::Null,
            4 => {
                let idx = read_u64(&mut data)? as usize;
                Constant::Closure(idx)
            }
            _ => return Err(format!("Unknown constant tag: {}", tag)),
        };
        constants.push(c);
    }

    let names_count = read_u64(&mut data)? as usize;
    let mut names = Vec::with_capacity(names_count);
    for _ in 0..names_count {
        let n_len = read_u64(&mut data)? as usize;
        if data.len() < n_len {
            return Err("Truncated name".to_string());
        }
        let (n_bytes, rest) = data.split_at(n_len);
        data = rest;
        names.push(Arc::from(String::from_utf8_lossy(n_bytes).into_owned()));
    }

    Ok(Chunk {
        code,
        constants,
        names,
        nested: vec![], // Nested chunks not yet supported in serialization
    })
}
