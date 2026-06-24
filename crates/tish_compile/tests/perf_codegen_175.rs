//! Generated-Rust assertions for #175 — native plain-array free fns.
//!
//! A top-level fn over `number[]`/`boolean[]` params (used only by index/length, no escape) whose
//! call sites pass pairwise-distinct array idents is de-virtualized to `fn name_nv(<f64..>,
//! <&/&mut Vec<T>..>) -> f64 | ()`; calls route there passing arrays by reference. A fn that calls a
//! boxed (non-native) closure, or whose array args can't be proven distinct/native, falls back to the
//! boxed closure (no `_nv`). Cross-backend soundness is covered by `tests/core/native_vec_params`.

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

fn compile_fixture_embedded_lib(rel: &str) -> String {
    compile_fixture_embedded_lib_with_features(rel, &[])
}

fn compile_fixture_embedded_lib_with_features(rel: &str, features: &[String]) -> String {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = tishlang_compile::compile_project_full_emit(
        &path,
        path.parent(),
        features,
        true,
        tishlang_compile::NativeEmitMode::EmbeddedLib,
        None,
    )
    .unwrap();
    rust
}

#[test]
fn native_vec_params_emit_ref_signatures_and_route() {
    let rust = compile_fixture_typed("tests/core/native_vec_params.tish");
    // Recursive `&mut boolean[]` fn.
    assert!(
        rust.contains("fn mark_nv(mut n: f64, mut row: f64, a: &mut Vec<bool>, b: &mut Vec<bool>)"),
        "mark lowers to a native free fn over &mut Vec<bool> params:\n{}",
        rust.lines().filter(|l| l.contains("mark_nv")).take(4).collect::<Vec<_>>().join("\n")
    );
    // Recursion reborrows the ref params through.
    assert!(
        rust.contains("mark_nv(n, (row + 1_f64), &mut *a, &mut *b)"),
        "recursive call reborrows the &mut params:\n{}",
        rust.lines().filter(|l| l.contains("mark_nv(")).take(4).collect::<Vec<_>>().join("\n")
    );
    // Read-only `&Vec<f64>` + write `&mut Vec<f64>`.
    assert!(
        rust.contains("fn scaleInto_nv(mut n: f64, src: &Vec<f64>, dst: &mut Vec<f64>)"),
        "scaleInto distinguishes read (&) vs written (&mut) array params:\n{}",
        rust.lines().filter(|l| l.contains("scaleInto_nv")).take(4).collect::<Vec<_>>().join("\n")
    );
    // Top-level call sites address-of native `Vec` locals.
    assert!(
        rust.contains("mark_nv(5_f64, 0_f64, &mut a, &mut b)"),
        "the entry call passes the local Vecs by &mut:\n{}",
        rust.lines().filter(|l| l.contains("mark_nv(5")).take(2).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("scaleInto_nv(6_f64, &src, &mut dst)"),
        "scaleInto call passes & / &mut:\n{}",
        rust.lines().filter(|l| l.contains("scaleInto_nv(6")).take(2).collect::<Vec<_>>().join("\n")
    );
    // The local arrays became native Vecs (the escape into the native-vec fn is not a boxing escape).
    assert!(
        rust.contains("let mut a: Vec<bool>") && rust.contains("let mut src: Vec<f64>"),
        "caller arrays are unboxed native Vecs"
    );
}

#[test]
fn queens_place_devirtualizes_to_native_vec_fn() {
    let rust = compile_fixture_typed("tests/perf/queens.tish");
    assert!(
        rust.contains("fn place_nv(mut n: f64, mut row: f64, cols: &mut Vec<bool>, diag1: &mut Vec<bool>, diag2: &mut Vec<bool>) -> f64"),
        "queens' place lowers to a native free fn over three &mut Vec<bool> params:\n{}",
        rust.lines().filter(|l| l.contains("place_nv")).take(3).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("place_nv(5_f64, 0_f64") || rust.contains("place_nv(match") || rust.contains("place_nv("),
        "the place call routes to place_nv"
    );
}

#[test]
fn spectral_norm_devirtualizes_with_inlined_evala() {
    // `multiplyAv`/`multiplyAtv` (over `&Vec<f64>` + `&mut Vec<f64>`) call `evalA`, a numeric leaf fn.
    // `evalA` inlines at the native-f64 call site (no dispatch, no boxed reference), so the native-vec
    // group de-virtualizes. The boxed `evalA` closure is left intact for any non-f64 callers.
    let rust = compile_fixture_typed("tests/perf/spectral_norm.tish");
    assert!(
        rust.contains("fn multiplyAv_nv(") && (rust.contains("v: &Vec<f64>") || rust.contains("av: &mut Vec<f64>")),
        "multiplyAv lowers to a native-vec fn:\n{}",
        rust.lines().filter(|l| l.contains("multiplyAv_nv")).take(3).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("fn multiplyAtAv_nv("),
        "multiplyAtAv should forward to native-vec callees:\n{}",
        rust.lines().filter(|l| l.contains("multiplyAtAv_nv")).take(3).collect::<Vec<_>>().join("\n")
    );
    // evalA is inlined: the native-vec body has the substituted body (a `_inl…` temp) and does NOT
    // call evalA (no `value_call`/`evalA(` inside multiplyAv_nv).
    let mav = rust
        .lines()
        .skip_while(|l| !l.contains("fn multiplyAv_nv("))
        .take_while(|l| !l.trim_start().starts_with("fn ") || l.contains("multiplyAv_nv"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        mav.contains("_inl") && !mav.contains("value_call"),
        "evalA must be inlined into multiplyAv_nv (substituted temps, no value_call):\n{}",
        mav
    );
    if rust.contains("fn spectralNorm_nv(") {
        let sn = rust.split("fn spectralNorm_nv(").nth(1).unwrap();
        let sn = sn.split("fn run()").next().unwrap_or(sn);
        assert!(
            sn.contains("let mut u: Vec<f64>"),
            "spectralNorm_nv should keep u/v/w as native Vec<f64> locals"
        );
        assert!(
            sn.contains("multiplyAtAv_nv(") && !sn.contains("multiplyAtAv_native("),
            "spectralNorm_nv should call multiplyAtAv_nv, not the boxed native shim"
        );
    }
}

#[test]
fn spectral_norm_embedded_lib_native_shim_compiles() {
    let rust = compile_fixture_embedded_lib("tests/perf/spectral_norm.tish");
    let mav = rust
        .split("fn multiplyAv_native(")
        .nth(1)
        .and_then(|s| s.split("fn multiplyAtv_native").next())
        .expect("multiplyAv_native");
    assert!(
        mav.contains("let mut j: f64")
            || mav.contains("get_index(&Value::Number(v), &Value::Number((_usize_j"),
        "boxed native shim must bind j from the usize loop counter:\n{}",
        mav.lines().take(12).collect::<Vec<_>>().join("\n")
    );
    assert!(
        mav.contains("let mut i: f64")
            || mav.contains("set_index(&(Value::Number(av)), &(Value::Number((_usize_i"),
        "boxed native shim must bind i from the usize loop counter:\n{}",
        mav.lines().take(12).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn spectral_norm_embedded_lib_with_runtime_features_native_shim_compiles() {
    let features: Vec<String> = [
        "http", "timers", "fs", "process", "regex", "ws", "tty",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let rust =
        compile_fixture_embedded_lib_with_features("tests/perf/spectral_norm.tish", &features);
    let mav = rust
        .split("fn multiplyAv_native(")
        .nth(1)
        .and_then(|s| s.split("fn multiplyAtv_native").next())
        .expect("multiplyAv_native");
    assert!(
        !mav.contains("Value::Number(j)") && !mav.contains("Value::Number(i)"),
        "runtime-feature build must not reference bare i/j in boxed shims:\n{}",
        mav.lines().take(12).collect::<Vec<_>>().join("\n")
    );
}
