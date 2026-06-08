//! Export the `tish_value_*` C-ABI accessors (from `tishlang_ffi`) in the `tish` binary's dynamic
//! symbol table, so a `dlopen`'d native extension (`ffi:`) can resolve them at load time. This is
//! the **decoupled** FFI model: the extension declares the accessors `extern "C"` and does NOT link
//! `tish_core`, so there is a single value representation and no host/extension layout matching.
//! Without this flag the host's accessor symbols aren't visible to the loaded library.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        // macOS/iOS: export all global symbols (so the two-level-namespace binary exposes them).
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-export_dynamic");
        }
        // Windows PE export tables work differently; the host-export FFI model isn't wired there yet.
        "windows" => {}
        // Linux/BSD: -rdynamic puts all symbols in the dynamic table.
        _ => {
            println!("cargo:rustc-link-arg-bins=-rdynamic");
        }
    }
}
