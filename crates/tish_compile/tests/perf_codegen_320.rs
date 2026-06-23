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

/// #320 part 2 (TISH_NATIVE_ARR_PARAM): a read-only `number[]` param of a normal (boxed-body,
/// boxed-return) fn is unboxed once into an owned native `Vec<f64>`, so the body indexes it
/// natively. This is the cross-fn-boundary lever — `kNucleotide(seq, k)` indexes `seq` but builds
/// & returns a boxed Map, so it is NOT a native-vec fn.
fn enable_arr_param() {
    enable_typed_flags();
    std::env::set_var("TISH_NATIVE_ARR_PARAM", "1");
}

const ARR_PARAM: &str = r#"
let seed = 7
function nextBase() {
  seed = (seed * 3877 + 29573) % 139968
  if (seed < 34992) { return 0 }
  if (seed < 69984) { return 1 }
  if (seed < 104976) { return 2 }
  return 3
}
function count(seq, k) {
  let m = new Map()
  let n = seq.length - k + 1
  for (let i = 0; i < n; i++) {
    let key = 0
    for (let j = 0; j < k; j++) { key = key * 4 + seq[i + j] }
    if (m.has(key)) { m.set(key, m.get(key) + 1) } else { m.set(key, 1) }
  }
  return m
}
let seq = []
for (let i = 0; i < 100; i++) { seq.push(nextBase()) }
let m = count(seq, 4)
let sum = 0
for (let v of m.values()) { sum = sum + v }
console.log("R " + sum)
"#;

#[test]
fn readonly_arr_param_unboxed_to_native_vec() {
    enable_arr_param();
    let rust = compile_src("arr_param", ARR_PARAM);
    // The `seq` param is unboxed once into an owned native Vec<f64> at the closure entry (a packed
    // NumberArray clones its backing; a boxed Array is mapped element-wise).
    assert!(
        rust.contains("let mut seq: Vec<f64> = match args.get(")
            && rust.contains("Some(Value::NumberArray(a)) => a.borrow().clone()"),
        "read-only number[] param should be unboxed to a native Vec<f64>"
    );
    // And the hot inner loop indexes it natively (f64 mul + native vec read), no boxed get_index.
    assert!(
        rust.contains("key * 4_f64") && rust.contains("seq.get("),
        "inner loop should be native f64 arithmetic over a native Vec read"
    );
    assert!(
        !rust.contains("tishlang_runtime::get_index(&seq")
            && !rust.contains("get_index(&(seq)"),
        "seq must not be read via the boxed runtime get_index inside the fn"
    );
}

#[test]
fn mutating_arr_param_stays_boxed() {
    enable_arr_param();
    // `a` is index-ASSIGNED → an owned native copy would silently drop the write, so the param must
    // stay boxed (classify_vec_param reports is_mut, and the detection requires read-only).
    let src = r#"
function bump(a) { a[0] = a[0] + 1; return a.length }
let arr = []
for (let i = 0; i < 10; i++) { arr.push(i) }
let r = bump(arr)
console.log("R " + r)
"#;
    let rust = compile_src("mut_arr_param", src);
    assert!(
        !rust.contains("let mut a: Vec<f64>"),
        "a mutated array param must stay boxed (soundness — owned copy would lose writes)"
    );
}

#[test]
fn escaping_arr_param_stays_boxed() {
    enable_arr_param();
    // `seq.push` MUTATES the param via a method (not index-assign): an owned copy would lose the
    // caller-visible push. `scan_param_use` flags this as an escape → the param stays boxed.
    let src = r#"
function build(seq) {
  let m = new Map()
  for (let i = 0; i < seq.length; i++) { m.set(i, seq[i]) }
  seq.push(99)
  return m
}
let arr = []
for (let i = 0; i < 10; i++) { arr.push(i) }
let m = build(arr)
console.log("R " + arr.length)
"#;
    let rust = compile_src("escape_arr_param", src);
    assert!(
        !rust.contains("let mut seq: Vec<f64>"),
        "an escaping (pushed) array param must stay boxed (soundness)"
    );
}

#[test]
fn non_number_array_arg_keeps_callee_boxed() {
    enable_arr_param();
    // `joinup` reads its param read-only, but the ONLY caller passes a NON-number[] array. Unboxing
    // to Vec<f64> would turn "x" into NaN (vs JS string concat) — so the call-site check keeps the
    // callee fully boxed.
    let src = r#"
function joinup(seq) { let r = ""; for (let i = 0; i < seq.length; i++) { r = r + seq[i] } return r }
let arr = [1, "x", 3]
console.log(joinup(arr))
"#;
    let rust = compile_src("nonnum_arr_arg", src);
    assert!(
        !rust.contains("let mut seq: Vec<f64>"),
        "a read-only fn whose caller passes a non-number[] array must stay boxed (call-site soundness)"
    );
}
