//! Generated-Rust assertions for #173 part 3 — in-bounds index elision.
//!
//! When a native `Vec` is filled to a fixed length and an index is proven `< len` (an enclosing
//! `i < len` guard) and `>= 0`, the store drops its OOB-growth `resize` branch and the read drops its
//! `.get().unwrap_or(..)` branch — a direct `a[i]` like V8/Bun emit after range-proving a loop. Any
//! array that can't be proven (escapes, is reassigned, or the index isn't a guarded counter) keeps
//! the OOB-safe lowering. Soundness across backends is covered by the `tests/core` parity corpus.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn enable_typed_flags() {
    for k in [
        "TISH_PARAM_NATIVE",
        "TISH_PARAM_INFER",
        "TISH_NATIVE_FN",
        "TISH_STRUCT_INFER",
        "TISH_FUSED_HOF",
        "TISH_NATIVE_HOF",
        "TISH_AGGREGATE_INFER",
    ] {
        std::env::set_var(k, "1");
    }
}

fn compile_fixture_typed(rel: &str) -> String {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

#[test]
fn nsieve_inbounds_store_and_read_elide_their_guards() {
    // `isPrime` is filled to length `n` and only indexed; the read `isPrime[i]` (guard `i < n`) and
    // the strided store `isPrime[k] = false` (guard `k < n`, `k` non-negative, stored before its own
    // `k = k + i` reassignment) are both provably in-bounds.
    let rust = compile_fixture_typed("tests/perf/nsieve.tish");
    // Read: direct index, NOT `.get(..).copied().unwrap_or(false)`.
    assert!(
        rust.contains("isPrime[(i) as usize]"),
        "in-bounds read should be a direct index:\n{}",
        rust.lines().filter(|l| l.contains("isPrime")).take(8).collect::<Vec<_>>().join("\n")
    );
    // Store: direct index, NO `resize` grow branch.
    assert!(
        rust.contains("{ isPrime[(k) as usize] = false; Value::Null }"),
        "in-bounds store should skip the resize-grow branch:\n{}",
        rust.lines().filter(|l| l.contains("isPrime")).take(8).collect::<Vec<_>>().join("\n")
    );
    assert!(
        !rust.contains("isPrime.resize") && !rust.contains("isPrime.get("),
        "the proven-fixed-length isPrime must have no resize/get fallbacks left:\n{}",
        rust.lines().filter(|l| l.contains("isPrime")).take(8).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn escaping_array_keeps_oob_safe_lowering() {
    // `a` is passed to a function, so it could be mutated/shrunk out of line — the fixed-length fact
    // must NOT apply and the store must keep its OOB-growth `resize` branch.
    let rust = compile_fixture_typed("tests/core/inbounds_index.tish");
    assert!(
        rust.contains("resize"),
        "an escaping array must retain the OOB-safe resize store:\n{}",
        rust.lines().filter(|l| l.contains("resize") || l.contains("esc")).take(8).collect::<Vec<_>>().join("\n")
    );
}
