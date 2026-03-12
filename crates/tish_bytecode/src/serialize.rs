//! Chunk serialization for embedding in native/WASM outputs.
//! Format: code, constants, names, nested (recursive).

use std::sync::Arc;

use super::{Chunk, Constant};

/// Serialize a chunk to bytes (includes nested chunks for functions).
pub fn serialize(chunk: &Chunk) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(chunk.code.len() as u64).to_le_bytes());
    out.extend_from_slice(&chunk.code);
    out.extend_from_slice(&(chunk.constants.len() as u64).to_le_bytes());
    for c in &chunk.constants {
        match c {
            Constant::Number(n) => {
                out.push(0);
                out.extend_from_slice(&n.to_le_bytes());
            }
            Constant::String(s) => {
                out.push(1);
                let b = s.as_bytes();
                out.extend_from_slice(&(b.len() as u64).to_le_bytes());
                out.extend_from_slice(b);
            }
            Constant::Bool(b) => {
                out.push(2);
                out.push(if *b { 1 } else { 0 });
            }
            Constant::Null => out.push(3),
            Constant::Closure(idx) => {
                out.push(4);
                out.extend_from_slice(&(*idx as u64).to_le_bytes());
            }
        }
    }
    out.extend_from_slice(&(chunk.names.len() as u64).to_le_bytes());
    for n in &chunk.names {
        let b = n.as_bytes();
        out.extend_from_slice(&(b.len() as u64).to_le_bytes());
        out.extend_from_slice(b);
    }
    out.extend_from_slice(&(chunk.nested.len() as u64).to_le_bytes());
    for nested in &chunk.nested {
        let nested_bytes = serialize(nested);
        out.extend_from_slice(&(nested_bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(&nested_bytes);
    }
    out.extend_from_slice(&chunk.rest_param_index.to_le_bytes());
    out
}

/// Deserialize a chunk from bytes.
pub fn deserialize(mut data: &[u8]) -> Result<Chunk, String> {
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

    let nested_count = read_u64(&mut data)? as usize;
    let mut nested = Vec::with_capacity(nested_count);
    for _ in 0..nested_count {
        let nested_len = read_u64(&mut data)? as usize;
        if data.len() < nested_len {
            return Err("Truncated nested chunk".to_string());
        }
        let (nested_data, rest) = data.split_at(nested_len);
        data = rest;
        nested.push(deserialize(nested_data)?);
    }

    let rest_param_index = if data.len() >= 2 {
        let (r_bytes, _) = data.split_at(2);
        u16::from_le_bytes(r_bytes.try_into().unwrap())
    } else {
        super::NO_REST_PARAM
    };

    Ok(Chunk {
        code,
        constants,
        names,
        nested,
        rest_param_index,
    })
}
