//! Generated-Rust assertions for #176 — native numeric globals (`thread_local Cell<f64>`).

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn enable_typed_flags() {
}

fn compile_fixture_typed(rel: &str) -> String {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

#[test]
fn fasta_seed_lowers_to_thread_local_and_genrandom_native() {
    let rust = compile_fixture_typed("tests/perf/fasta.tish");
    assert!(
        rust.contains("thread_local!"),
        "fasta emits thread_local globals:\n{}",
        rust.lines().filter(|l| l.contains("thread_local") || l.contains("G_SEED")).take(6).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("static G_SEED: std::cell::Cell<f64>"),
        "seed lowers to G_SEED Cell:\n{}",
        rust.lines().filter(|l| l.contains("G_SEED")).take(4).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("fn genRandom_native("),
        "genRandom becomes M5-eligible after native global seed:\n{}",
        rust.lines().filter(|l| l.contains("genRandom")).take(6).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("G_SEED.with(|c| c.set("),
        "genRandom_native mutates G_SEED:\n{}",
        rust.lines().filter(|l| l.contains("G_SEED.with")).take(6).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn native_numeric_global_fixture_interleaved_state() {
    let rust = compile_fixture_typed("tests/core/native_numeric_global.tish");
    assert!(
        rust.contains("static G_COUNTER: std::cell::Cell<f64>"),
        "counter global lowers:\n{}",
        rust.lines().filter(|l| l.contains("G_COUNTER")).take(4).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("G_COUNTER.with(|c| c.get())") && rust.contains("G_COUNTER.with(|c| c.set("),
        "boxed + native paths share G_COUNTER:\n{}",
        rust.lines()
            .filter(|l| l.contains("G_COUNTER::with"))
            .take(8)
            .collect::<Vec<_>>()
            .join("\n")
    );
}
