//! tishlang/tish#558 — in-place native `Vec` mutators (`pop`/`shift`/`unshift`/`reverse`/`sort`) must
//! mutate the REAL Vec, both for a plain local and for a closure-captured (refcell-wrapped) one. The
//! boxed `array_*` fallback operates on a THROWAWAY boxed copy of the Vec (cloning + `from_struct`-boxing
//! every element into a `Value::Array`), so the mutation is silently discarded (O(n), and an E0282 on
//! the boxing closure for a `Vec<struct>`). These assert the native forms are emitted — plain (`a.pop()`,
//! `a.remove(0)`, `a.insert(0, …)`, `a.reverse()`, `a.sort_by(…)`) and captured (`(*a.borrow_mut())…`) —
//! and that the throwaway-boxing fallback does NOT appear.
//!
//! End-to-end runtime parity (interp/vm == native) is exercised by the fixtures under tests/core via
//! `test_mvp_programs_native`; these codegen-string checks pin the lowering itself.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn emit(body: &str, name: &str) -> String {
    let src = format!("interface E {{ x: number; y: number }}\n{}", body);
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, src).unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

// A plain local `a` mutated at top level (not captured) — native, not wrapped.
const PLAIN: &str = "\
let a: E[] = []
a.push({ x: 1, y: 2 })
a.push({ x: 3, y: 4 })
let p = a.pop()
let s = a.shift()
a.unshift({ x: 9, y: 9 })
a.reverse()
a.sort((u, v) => u.x - v.x)
console.log(a.length)
";

#[test]
fn plain_native_vec_mutators_are_native() {
    let r = emit(PLAIN, "plain_mut.tish");
    assert!(r.contains("a.pop()"), "pop must be native `a.pop()`\n{r}");
    assert!(r.contains("a.remove(0)"), "shift must be native `a.remove(0)`\n{r}");
    assert!(r.contains("a.insert(0,"), "unshift must be native `a.insert(0, …)`\n{r}");
    assert!(r.contains("a.reverse()"), "reverse must be native `a.reverse()`\n{r}");
    assert!(r.contains("a.sort_by("), "sort must be native `a.sort_by(…)`\n{r}");
    // None of the mutators may fall to the throwaway whole-Vec boxing fallback.
    for boxed in ["array_pop(&Value::Array(VmRef::new", "array_shift", "array_unshift", "array_reverse"] {
        assert!(!r.contains(boxed), "mutator must not box the whole Vec (`{boxed}`)\n{r}");
    }
}

// `a` is captured AND mutated inside a closure → refcell-wrapped. Each mutator must go through the cell.
fn wrapped(mutator: &str, name: &str) -> String {
    emit(
        &format!(
            "let a: E[] = []\na.push({{ x: 1, y: 2 }})\na.push({{ x: 3, y: 4 }})\nfunction f() {{ {mutator} }}\nf()\nconsole.log(a.length)\n"
        ),
        name,
    )
}

#[test]
fn wrapped_pop_is_native_through_cell() {
    let r = wrapped("a.pop()", "w_pop.tish");
    assert!(r.contains(".borrow_mut()).pop()"), "wrapped pop must be `(*a.borrow_mut()).pop()`\n{r}");
    assert!(!r.contains("array_pop(&Value::Array(VmRef::new"), "wrapped pop must not box the whole Vec\n{r}");
}

#[test]
fn wrapped_shift_is_native_through_cell() {
    let r = wrapped("a.shift()", "w_shift.tish");
    assert!(r.contains(".borrow_mut()).remove(0)"), "wrapped shift must be `(*a.borrow_mut()).remove(0)`\n{r}");
}

#[test]
fn wrapped_unshift_is_native_through_cell() {
    let r = wrapped("a.unshift({ x: 9, y: 9 })", "w_unshift.tish");
    assert!(r.contains(".borrow_mut()).insert(0,"), "wrapped unshift must be `(*a.borrow_mut()).insert(0, …)`\n{r}");
}

#[test]
fn wrapped_reverse_is_native_through_cell() {
    let r = wrapped("a.reverse()", "w_reverse.tish");
    assert!(r.contains(".borrow_mut()).reverse()"), "wrapped reverse must be `(*a.borrow_mut()).reverse()`\n{r}");
}

#[test]
fn wrapped_sort_is_native_through_cell() {
    let r = wrapped("a.sort((u, v) => u.x - v.x)", "w_sort.tish");
    assert!(
        r.contains(".borrow_mut()).sort_by(") || r.contains(".borrow_mut() = "),
        "wrapped sort must mutate through the cell (`(*a.borrow_mut()).sort_by` or a write-back)\n{r}"
    );
}

// ── splice / fill / copyWithin (native emitters, plain + captured) ──

const SFC_PLAIN: &str = "\
let a: E[] = []
a.push({ x: 1, y: 2 })
a.push({ x: 3, y: 4 })
let removed = a.splice(0, 1, { x: 9, y: 9 })
a.fill({ x: 0, y: 0 }, 0, 1)
a.copyWithin(0, 1)
console.log(a.length + \"\" + removed.length)
";

#[test]
fn plain_splice_fill_copywithin_are_native() {
    let r = emit(SFC_PLAIN, "sfc_plain.tish");
    assert!(r.contains(".splice(__start"), "splice must be native `Vec::splice`\n{r}");
    assert!(r.contains("__v.clone()"), "fill must write elements natively\n{r}");
    assert!(r.contains(".to_vec()"), "copyWithin must copy via a native temp\n{r}");
    for boxed in ["array_splice", "array_fill", "array_copy_within", "copy_within as"] {
        assert!(!r.contains(boxed), "must not fall to the boxed `{boxed}` on a throwaway copy\n{r}");
    }
}

#[test]
fn wrapped_splice_is_native_through_guard() {
    // `a` captured + spliced in a closure → wrapped; splice uses a `borrow_mut` guard.
    let r = wrapped("a.splice(0, 1)", "w_splice.tish");
    assert!(
        r.contains(".borrow_mut()") && r.contains("__g.splice("),
        "wrapped splice must run on a `borrow_mut` guard (`__g.splice(..)`)\n{r}"
    );
    assert!(!r.contains("array_splice"), "wrapped splice must not box the whole Vec\n{r}");
}

// ── captured (wrapped) array READS must NOT box the whole Vec (tish#558 read side) ──

#[test]
fn wrapped_element_field_read_is_native_not_whole_array_box() {
    // `arr` captured + mutated in a closure → wrapped; reading `arr[0].x` must index the borrowed
    // Vec directly, NOT clone+box the entire Vec into a `Value::Array` just to read one field.
    let src = "let arr: E[] = []\narr.push({ x: 1, y: 2 })\nfunction f() { arr[0].x = 9; return arr[0].x }\nconsole.log(f())\n";
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("w_read.tish");
    std::fs::write(&path, format!("interface E {{ x: number; y: number }}\n{src}")).unwrap();
    let r = compile_project_full(&path, path.parent(), &[], true).unwrap().0;
    assert!(
        r.contains("(*arr.borrow())["),
        "wrapped `arr[i].field` read must index the borrowed Vec natively\n{r}"
    );
    assert!(
        !r.contains("iter().cloned().map(|v| tishlang_runtime::Value::from_struct"),
        "wrapped read must NOT clone+box the whole Vec into a Value::Array (O(n)/read)\n{r}"
    );
}

#[test]
fn push_returns_new_length_not_null() {
    // `push` returns the new length (JS / tish interp), a plain `.len()` read — not `Value::Null`.
    let r = emit("let a: E[] = []\nlet n = a.push({ x: 1, y: 2 })\nconsole.log(n)\n", "push_ret.tish");
    assert!(
        r.contains(".len() as f64"),
        "push must return the new length (`.len() as f64`), not Null\n{r}"
    );
}
