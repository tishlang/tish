
fn main() {
    let chunk = include_bytes!("../chunk.bin");
    if let Err(e) = tishlang_wasm_runtime::run_wasi(chunk) {
        eprintln!("Runtime error: {}", e);
        std::process::exit(1);
    }
}
