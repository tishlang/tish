//! `tish build --target rust-lib` end to end: the emitted artifact is a Cargo **source crate**, and
//! cargo is deliberately not run — the consumer decides what to build it for.
//!
//! The fast test pins the crate layout. The behavioural test — module state surviving between
//! `pub fn` calls, a `clear*()` reassignment being visible to other exports, and the module body
//! running exactly once — actually builds the crate and a consumer, so it is gated behind
//! `TISH_SLOW_TESTS=1` to keep the default `cargo test` fast.
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/regression/rust_lib_emit.tish")
        .canonicalize()
        .unwrap()
}

fn emit_crate(out: &Path) {
    let entry = fixture();
    tishlang_native::compile_to_native_with_config(
        &entry,
        entry.parent(),
        out,
        &[],
        "rust",
        true,
        &tishlang_native::NativeBuildConfig::rust_lib(),
    )
    .expect("rust-lib emit");
}

#[test]
fn emits_an_rlib_source_crate_and_does_not_run_cargo() {
    let out = std::env::temp_dir().join("tish_rustlib_layout");
    let _ = std::fs::remove_dir_all(&out);
    emit_crate(&out);

    let manifest = std::fs::read_to_string(out.join("Cargo.toml")).expect("Cargo.toml");
    assert!(
        manifest.contains(r#"crate-type = ["rlib"]"#),
        "expected an rlib library crate:\n{manifest}"
    );
    assert!(
        manifest.contains("path = \"src/lib.rs\""),
        "expected a lib entry point:\n{manifest}"
    );
    // Both `path` and `version` on the runtime dep: cargo prefers the path locally and falls back to
    // the registry when the emitted crate is published, so it is publishable as-is.
    assert!(
        manifest.contains("tishlang_runtime = { path =") && manifest.contains("version = \""),
        "the runtime dep must carry both path and version:\n{manifest}"
    );

    assert!(out.join("src/lib.rs").is_file(), "expected src/lib.rs");
    assert!(
        !out.join("src/main.rs").exists(),
        "a library crate must not emit src/main.rs"
    );
    // Cargo was not invoked: no build output next to the emitted crate.
    assert!(
        !out.join("target").exists(),
        "--target rust-lib must emit the crate and stop, not build it"
    );
}

/// `TISH_SLOW_TESTS=1 cargo test -p tishlang --test rust_lib_crate` — compiles the emitted crate
/// plus a consumer that exercises the module-state guarantees.
#[test]
fn consumer_observes_persistent_module_state() {
    if std::env::var("TISH_SLOW_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped (set TISH_SLOW_TESTS=1 to run the full cargo build)");
        return;
    }
    let root = std::env::temp_dir().join("tish_rustlib_e2e");
    let _ = std::fs::remove_dir_all(&root);
    let lib = root.join("lib");
    emit_crate(&lib);

    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        format!(
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [workspace]\n\n[dependencies]\nlib = {{ package = \"lib\", path = {:?} }}\n",
            lib.display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(
        consumer.join("src/main.rs"),
        r#"
use lib::Value;
fn num(v: &Value) -> f64 { match v { Value::Number(x) => *x, _ => f64::NAN } }
fn text(v: &Value) -> String { match v { Value::String(x) => x.to_string(), _ => String::new() } }
fn main() {
    // First call is not the first declared export: init must already have run.
    assert_eq!(num(&lib::add(Value::Number(2.0), Value::Number(40.0))), 42.0, "add");
    assert_eq!(num(&lib::initRuns()), 1.0, "module top level runs once");

    lib::register(Value::String("kick".into()), Value::String("noiseBurst".into()));
    assert_eq!(text(&lib::lookup(Value::String("kick".into()))), "noiseBurst", "state persists");

    // `clearAll` REASSIGNS the module-level binding; other exports must see that.
    assert_eq!(num(&lib::clearAll()), 0.0, "clearAll empties");
    assert_eq!(text(&lib::lookup(Value::String("kick".into()))), "miss", "reassignment visible");

    assert_eq!(num(&lib::initRuns()), 1.0, "still initialised once");

    // A throw is parked for the next checkpoint, not unwound. Across the `pub fn` boundary there is
    // no Tish frame to surface it in, so a parked throw used to sit latched and make every LATER
    // call bail and return null. Reading a property off a null is all it takes.
    let _ = lib::runtime::get_index(&Value::Null, &Value::String("nope".into()));
    assert_eq!(
        num(&lib::add(Value::Number(1.0), Value::Number(2.0))),
        3.0,
        "a parked throw must not brick the module"
    );
    for _ in 0..20 {
        let _ = lib::runtime::get_index(&Value::Null, &Value::String("nope".into()));
    }
    assert_eq!(text(&lib::lookup(Value::String("kick".into()))), "miss", "still working after 20");

    println!("ALL_OK");
}
"#,
    )
    .unwrap();

    let out = std::process::Command::new("cargo")
        .args(["run", "-q"])
        .current_dir(&consumer)
        .output()
        .expect("cargo run");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ALL_OK"),
        "consumer failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
