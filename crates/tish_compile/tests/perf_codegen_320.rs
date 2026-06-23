//! #320 — honest native-array lowering for fn-call-built numeric arrays.
//!
//! Two sound, name-independent inference fixes (NO `TISH_GAUNTLET_FUSION`):
//!   fix1: a 0-param numeric fn (`nextBase`, mutates a numeric global, returns number) is eligible
//!         for native `-> f64` promotion (`collect_native_fns` no longer requires ≥1 param).
//!   fix2: a fn PROVEN to always return a number lets `infer_expr_type(f(...))` be `number`, so
//!         `seq.push(nextBase())` keeps `seq` a native `Vec<f64>` instead of a boxed `Value[]`.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

/// Honest typed flags — the fixture-substitution kernels stay OFF.
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
    std::env::remove_var("TISH_GAUNTLET_FUSION");
}

/// Write `src` under the workspace `target/` (scratch lives there, per repo convention) and compile.
fn compile_src(stem: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/perf_codegen_320");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{stem}.tish"));
    std::fs::write(&path, src).unwrap();
    let path = path.canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

const NON_ESCAPING: &str = r#"
let seed = 7
function nextBase() {
  seed = (seed * 3877 + 29573) % 139968
  if (seed < 34992) { return 0 }
  if (seed < 69984) { return 1 }
  if (seed < 104976) { return 2 }
  return 3
}
let seq = []
for (let i = 0; i < 100; i++) { seq.push(nextBase()) }
let sum = 0
for (let i = 0; i < seq.length; i++) { sum = sum + seq[i] }
console.log("R " + sum)
"#;

#[test]
fn zero_param_numeric_fn_goes_native() {
    enable_typed_flags();
    let rust = compile_src("zero_param", NON_ESCAPING);
    // fix1: the 0-param fn is promoted to a native `-> f64` form.
    assert!(
        rust.contains("fn nextBase_native() -> f64"),
        "0-param numeric fn should get a native f64 form (fix1)"
    );
}

#[test]
fn numeric_returning_fn_keeps_pushed_array_native() {
    enable_typed_flags();
    let rust = compile_src("native_push", NON_ESCAPING);
    // fix2: `seq.push(nextBase())` keeps `seq` a native `Vec<f64>` (the pushed value infers numeric).
    assert!(
        rust.contains("let mut seq: Vec<f64>"),
        "seq built by push(numericFn()) should be a native Vec<f64> (fix2)"
    );
    // And the push routes straight to the native fn (no boxed Value array push).
    assert!(
        rust.contains("seq.push(nextBase_native())"),
        "push should call the native fn into the native Vec (fix1+fix2)"
    );
}

/// A fn that can fall through to an implicit `undefined` (no trailing unconditional return) must NOT
/// be treated as number-returning — else `a.push(maybe())` could put `undefined` in a `Vec<f64>`.
#[test]
fn fallthrough_fn_is_not_number_returning() {
    enable_typed_flags();
    let src = r#"
let g = 0
function maybe() {
  g = g + 1
  if (g < 10) { return 1 }
}
let a = []
for (let i = 0; i < 100; i++) { a.push(maybe()) }
let sum = 0
for (let i = 0; i < a.length; i++) { sum = sum + 1 }
console.log("R " + sum)
"#;
    let rust = compile_src("fallthrough", src);
    // Unsound to type `a` as Vec<f64> here — it must stay a boxed array.
    assert!(
        !rust.contains("let mut a: Vec<f64>"),
        "fall-through fn must not make the pushed array native (soundness)"
    );
}
