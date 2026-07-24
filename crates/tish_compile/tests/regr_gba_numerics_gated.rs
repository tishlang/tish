//! Regression: `fixed` and the narrow integer widths (`i8/u8/i16/u16/u32`) are the GBA typed-scalar
//! vocabulary and must lower to native scalars ONLY for `--target gba`. On the default native build
//! they must fall back to boxed `Value` — otherwise `fixed` references `tishlang_runtime::Fixed`,
//! which does not exist off-GBA (a hard compile error), and narrow widths truncate on store,
//! diverging from the interpreter and breaking the typed == boxed == interpreter guarantee.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    // compile_project_full uses the default DesktopBin emit mode (i.e. NOT --target gba).
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

#[test]
fn narrow_int_and_fixed_do_not_lower_to_native_off_gba() {
    let rust = compile("tests/regression/gba_numerics_gated.tish");
    // `fixed` must not emit the GBA-only native type (absent off-GBA → would not compile).
    assert!(
        !rust.contains("tishlang_runtime::Fixed"),
        "`: fixed` must fall back to boxed Value on a non-GBA build (found the native Fixed type):\n{}",
        rust
    );
    // The narrow-width fields must not become native storage types (which would truncate).
    assert!(
        !rust.contains(": u8") && !rust.contains(": i16") && !rust.contains("as u8"),
        "narrow-int annotations must stay boxed Value off-GBA (found native narrow storage):\n{}",
        rust
    );
}
