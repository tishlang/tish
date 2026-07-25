//! Regression: `fixed` and the narrow integer widths (`i8/u8/i16/u16/u32`) are the GBA typed-scalar
//! vocabulary and must lower to native scalars ONLY for `--target gba`. On the default native build
//! they must fall back to boxed `Value` — otherwise `fixed` references `tishlang_runtime::Fixed`,
//! which does not exist off-GBA (a hard compile error), and narrow widths truncate on store,
//! diverging from the interpreter and breaking the typed == boxed == interpreter guarantee.
use std::path::PathBuf;
use tishlang_compile::{compile_project_full, compile_project_full_emit, NativeEmitMode};

fn fixture(rel: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../..").join(rel).canonicalize().unwrap()
}

fn compile(rel: &str) -> String {
    let path = fixture(rel);
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

#[test]
fn gba_numeric_profile_does_not_leak_to_a_later_desktop_compile() {
    // The numeric profile is a thread-local. A GBA compile turns it on; a subsequent non-GBA
    // compile on the SAME thread (as happens in an LSP / a `cargo test` worker) must NOT inherit it
    // — otherwise `: fixed`/narrow/`i32` would lower natively off-GBA and diverge. This orders both
    // compiles on one thread and checks the desktop one is clean.
    let path = fixture("tests/regression/gba_numerics_gated.tish");
    let gba = compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .unwrap()
        .0;
    assert!(
        gba.contains("tishlang_runtime::Fixed"),
        "sanity: GBA mode DOES lower `: fixed` to the native Fixed type"
    );
    // Same thread, right after the GBA compile: a default (DesktopBin) compile must reset the profile.
    let desktop = compile_project_full(&path, path.parent(), &[], true).unwrap().0;
    assert!(
        !desktop.contains("tishlang_runtime::Fixed"),
        "a desktop compile after a GBA compile leaked the GBA numeric profile (found native Fixed):\n{}",
        desktop
    );
}
