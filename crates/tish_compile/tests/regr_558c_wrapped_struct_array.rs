//! tishlang/tish#558 — a captured (refcell-wrapped) native struct `Vec` must mutate NATIVELY, not fall
//! to the boxed path. When a struct array is closure-captured-and-mutated it is wrapped in a `VmRef`;
//! the native fast paths used to bail on that (`!refcell_wrapped_vars`), so `arr[i].field = v` and
//! `arr.push(x)` fell to the generic boxed path, which cloned+`from_struct`-boxed the WHOLE Vec into a
//! throwaway `Value::Array` and ran `set_prop`/`array_push` on the discarded copy — O(n) per op, the
//! write lost, and (for a `Vec<struct>`) an E0282 on the boxing closure since the skipped native op
//! never pinned the element type. These assert the native `borrow_mut` forms are emitted instead.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn emit(src: &str, name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, src).unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

// `arr` is captured AND mutated (field-assign) by `update`, so it is refcell-wrapped.
const WRAPPED_ASSIGN: &str = "\
interface E { x: number; y: number }
let arr: E[] = []
arr.push({ x: 0, y: 0 })
function update() { arr[0].x = 200 }
update()
console.log(arr[0].x)
";

#[test]
fn wrapped_struct_element_field_assign_is_a_native_store() {
    let rust = emit(WRAPPED_ASSIGN, "wrapped_assign.tish");
    // Native element field store through the shared cell — not a `set_prop` on a boxed copy.
    assert!(
        rust.contains("borrow_mut())[_i]") || rust.contains(".borrow_mut())[") ,
        "#558: wrapped `arr[i].field = v` must store natively through `(*arr.borrow_mut())[i]`.\n{rust}"
    );
    assert!(
        !rust.contains("set_prop(&(tishlang_runtime::get_index(&Value::Array(VmRef::new"),
        "#558: wrapped struct assign must NOT box the whole Vec and set_prop a throwaway copy.\n{rust}"
    );
}

// `a` is captured AND mutated (push) by `f`, so it is refcell-wrapped.
const WRAPPED_PUSH: &str = "\
interface E { x: number; y: number }
let a: E[] = []
function f() { a.push({ x: 1, y: 2 }) }
f()
console.log(a[0].x)
";

#[test]
fn wrapped_struct_push_is_a_native_push() {
    let rust = emit(WRAPPED_PUSH, "wrapped_push.tish");
    // Native push through the shared cell — not a boxed `array_push` on a whole-Vec copy.
    assert!(
        rust.contains("borrow_mut()).push("),
        "#558: wrapped `a.push(struct)` must push natively through `(*a.borrow_mut()).push(..)`.\n{rust}"
    );
    assert!(
        !rust.contains("array_push(&Value::Array(VmRef::new"),
        "#558: wrapped struct push must NOT box the whole Vec into a throwaway array_push.\n{rust}"
    );
}
