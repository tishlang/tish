//! M5 native fns, with an `i32` calling convention.
//!
//! `collect_native_fns` gated parameters and returns on `RustType::F64`, so `: number` qualified for
//! a native free `fn` and `: i32` did not — and `: i32` is the annotation a device with no FPU
//! wants. Every argument and every return of such a call was therefore an i32→f64→i32 round trip,
//! which on ARM7TDMI is a soft-float call apiece.
//!
//! The shape is ALL-OR-NOTHING: every parameter and the return annotated `: i32`. Mixing is what
//! makes it unsound — an `: i32` parameter reached through the boxed path is ToInt32-coerced on
//! entry, and promoted to a native `f64` parameter it would not be, so 3.7 would arrive as 3 one
//! way and 3.7 the other.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

/// Compile for GBA — `: i32` is only a native scalar under the GBA numeric vocabulary
/// (`types.rs`: `"i32" if gba_numerics()`), so on a desktop target this ABI cannot exist and the
/// assertions below would be vacuous.
fn compile_src(stem: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/perf_codegen_i32_abi");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{stem}.tish"));
    std::fs::write(&path, src).unwrap();
    let path = path.canonicalize().unwrap();
    let (rust, _, _, _) =
        compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
            .unwrap();
    rust
}

const ALL_I32: &str = r#"
function mix(a: i32, b: i32): i32 { return ((a * 3) ^ (b << 2)) & 65535 }
let acc: i32 = 0
let j: i32 = 0
while (j < 10) { acc = (acc + mix(j, acc)) & 65535; j = j + 1 }
console.log(acc)
"#;

#[test]
fn an_all_i32_signature_gets_an_i32_native_fn() {
    let rust = compile_src("all_i32", ALL_I32);
    assert!(
        rust.contains("fn mix_native(mut a: i32, mut b: i32) -> i32"),
        "expected an i32-signature native fn, got:\n{}",
        rust.lines().filter(|l| l.contains("mix_native")).collect::<Vec<_>>().join("\n")
    );
    // And the call must reach it directly rather than through `value_call`.
    assert!(rust.contains("mix_native("), "the call site should target the native fn");
}

const MIXED: &str = r#"
function half(a: i32, b: number): i32 { return (a + b) | 0 }
console.log(half(7, 2.5))
"#;

#[test]
fn a_mixed_signature_does_not_get_the_i32_abi() {
    let rust = compile_src("mixed", MIXED);
    assert!(
        !rust.contains("fn half_native(mut a: i32"),
        "a mixed i32/number signature must not take the i32 ABI — an `: i32` param is \
         ToInt32-coerced on entry and an f64 native param is not"
    );
}

const LIAR: &str = r#"
function liar(x: i32): i32 {
  if (x > 0) { return "not a number" }
  return 7
}
console.log(liar(1))
"#;

#[test]
fn the_annotation_alone_does_not_qualify_a_fn() {
    // Return annotations are erased and unchecked: this program prints the STRING today. The i32
    // ABI must not be handed out on the strength of `: i32`, only on the fixpoint's proof that
    // every return is numeric — otherwise this would unbox a string as a number.
    let rust = compile_src("liar", LIAR);
    assert!(
        !rust.contains("fn liar_native"),
        "a fn with a non-numeric return path must stay boxed regardless of its annotation"
    );
}

const READS_MODULE_STATE: &str = r#"
let ARR: i32[] = []
let k: i32 = 0
while (k < 8) { ARR.push(k); k = k + 1 }
function pure(a: i32, b: i32): i32 { return (a ^ b) & 255 }
function reads(i: i32): i32 { return ARR[i] & 255 }
let x: i32 = pure(1, 2) + reads(3)
console.log(x)
"#;

#[test]
fn a_fn_reading_module_state_stays_boxed() {
    // A native fn is emitted at TOP LEVEL, where a module-scope `let` — a local of `run()`, closed
    // over by the boxed closures — does not exist. `native_safe_expr`'s index arm accepted any
    // identifier as the indexed object, so this compiled to a `reads_native` whose body named
    // `ARR` and rustc rejected the generated program with E0425. The sibling `pure` must still be
    // promoted: the check has to reject the out-of-scope read, not indexing as such.
    let rust = compile_src("reads_module_state", READS_MODULE_STATE);
    assert!(
        !rust.contains("fn reads_native"),
        "a fn indexing a module-scope binding cannot become a top-level free fn"
    );
    assert!(
        rust.contains("fn pure_native(mut a: i32, mut b: i32) -> i32"),
        "its parameter-only sibling must still get the native i32 fn"
    );
}
