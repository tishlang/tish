//! Bytecode to Cranelift IR lowering.
//!
//! Emits object file with tish_chunk_data and tish_chunk_len symbols.
//! The link step builds a Rust binary that reads these and runs via tish_vm.

use std::path::Path;

use cranelift::codegen::settings::Configurable;
use cranelift::codegen::settings;
use cranelift_module::{DataDescription, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use tish_bytecode::{Chunk, Constant};

use crate::CraneliftError;

pub fn lower_and_emit(chunk: &Chunk, object_path: &Path) -> Result<(), CraneliftError> {
    let mut settings_builder = settings::builder();
    settings_builder.set("opt_level", "speed").map_err(|_| CraneliftError {
        message: "Failed to set opt_level".to_string(),
    })?;
    let flags = settings::Flags::new(settings_builder);

    let isa_builder = cranelift_native::builder().map_err(|e| CraneliftError {
        message: format!("Failed to build ISA: {}", e),
    })?;
    let isa = isa_builder.finish(flags).map_err(|e| CraneliftError {
        message: format!("Failed to finish ISA: {}", e),
    })?;

    let object_builder = ObjectBuilder::new(isa, "tish_cranelift", cranelift_module::default_libcall_names())
        .map_err(|e| CraneliftError {
            message: format!("Failed to create ObjectBuilder: {}", e),
        })?;
    let mut module = ObjectModule::new(object_builder);

    // Serialize chunk and emit as data - link step will build a Rust binary that reads it
    let chunk_data = serialize_chunk(chunk);
    let chunk_len = chunk_data.len() as u64;
    let data_id = module
        .declare_data("tish_chunk_data", Linkage::Export, false, false)
        .map_err(|e| CraneliftError {
            message: format!("Failed to declare chunk data: {}", e),
        })?;
    let mut data_desc = DataDescription::new();
    data_desc.define(chunk_data.into_boxed_slice());
    module
        .define_data(data_id, &data_desc)
        .map_err(|e| CraneliftError {
            message: format!("Failed to define chunk data: {}", e),
        })?;

    let len_data = chunk_len.to_le_bytes();
    let len_id = module
        .declare_data("tish_chunk_len", Linkage::Export, false, false)
        .map_err(|e| CraneliftError {
            message: format!("Failed to declare chunk len: {}", e),
        })?;
    let mut len_desc = DataDescription::new();
    len_desc.define(len_data.to_vec().into_boxed_slice());
    module
        .define_data(len_id, &len_desc)
        .map_err(|e| CraneliftError {
            message: format!("Failed to define chunk len: {}", e),
        })?;

    let object_product = module.finish();
    let bytes = object_product.emit().map_err(|e| CraneliftError {
        message: format!("Failed to emit object: {}", e),
    })?;

    std::fs::write(object_path, bytes).map_err(|e| CraneliftError {
        message: format!("Failed to write object file: {}", e),
    })?;

    Ok(())
}

fn serialize_chunk(chunk: &Chunk) -> Vec<u8> {
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
    out
}
