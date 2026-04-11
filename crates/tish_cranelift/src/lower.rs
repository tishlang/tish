//! Embed serialized bytecode in an object file for the standalone native binary.
//!
//! **This is not AOT compilation of Tish into Cranelift IR.** The chunk is stored as
//! read-only data (`tish_chunk_data`, `tish_chunk_len`). The link step produces an
//! executable that **deserializes the chunk and runs `tishlang_vm`** — same VM as
//! `tish run --backend vm`. Cranelift is only the object-file emitter for that blob.

use std::path::Path;

use cranelift::codegen::settings;
use cranelift::codegen::settings::Configurable;
use cranelift_module::{DataDescription, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use tishlang_bytecode::{serialize, Chunk};

use crate::CraneliftError;

pub fn lower_and_emit(chunk: &Chunk, object_path: &Path) -> Result<(), CraneliftError> {
    let mut settings_builder = settings::builder();
    settings_builder
        .set("opt_level", "speed")
        .map_err(|_| CraneliftError {
            message: "Failed to set opt_level".to_string(),
        })?;
    let flags = settings::Flags::new(settings_builder);

    let isa_builder = cranelift_native::builder().map_err(|e| CraneliftError {
        message: format!("Failed to build ISA: {}", e),
    })?;
    let isa = isa_builder.finish(flags).map_err(|e| CraneliftError {
        message: format!("Failed to finish ISA: {}", e),
    })?;

    let object_builder = ObjectBuilder::new(
        isa,
        "tishlang_cranelift",
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| CraneliftError {
        message: format!("Failed to create ObjectBuilder: {}", e),
    })?;
    let mut module = ObjectModule::new(object_builder);

    // Serialize chunk and emit as data - link step will build a Rust binary that reads it
    let chunk_data = serialize(chunk);
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
