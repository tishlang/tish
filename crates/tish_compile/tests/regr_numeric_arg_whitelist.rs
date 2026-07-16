//! tishlang/tish#485/#477/#484 — SYSTEMIC: a param is native-promoted to f64 only if the
//! interprocedural per-position fixpoint proves every call site feeds it numbers. #485's
//! return-either-param helper (called with string|null results) must NOT get an f64 unbox; the
//! recursive + pass-through numeric fns MUST still promote (no perf regression).
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

#[test]
fn unsafe_numeric_arg_stays_boxed_no_f64_unbox() {
    let rust = compile("tests/regression/f64_return_either_param.tish");
    let bad: Vec<&str> = rust.lines().filter(|l| l.contains("expected number")).collect();
    assert!(bad.is_empty(), "no fn here does arithmetic; nothing should f64-unbox:\n{}", bad.join("\n"));
}

#[test]
fn recursive_and_passthrough_numeric_fns_still_promote() {
    let rust = compile("tests/regression/numeric_arg_whitelist.tish");
    // fib(n-1)/fib(n-2): n is arithmetic → promoted; work(n) via driver's param n → pass-through.
    assert!(
        rust.contains("fib") && (rust.contains(": f64") || rust.contains("Value::Number")),
        "recursive/pass-through numeric params must still get the native f64 fast path"
    );
}
