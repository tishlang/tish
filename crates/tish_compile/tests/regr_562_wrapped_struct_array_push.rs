//! tishlang/tish#562 — `.push()` (and `.length`) on a captured (refcell-wrapped) typed struct array
//! must lower to NATIVE ops, not box the whole `Vec` into a `Value::Array` via an UNTYPED `map(|v| …)`
//! closure. The boxed path both wasted O(n) per op AND failed to compile with `error[E0282]: type
//! annotations needed` (the untyped closure param, since the skipped native push never pinned the Vec's
//! element type). Reproduced on the GBA emit path (the `i32` struct field is native only there).

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn gba_emit(src: &str, name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, src).unwrap();
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .unwrap()
        .0
}

// The #562 repro shape: a typed struct array with an i32 field, captured + pushed in a closure (which
// is what refcell-wraps it, matching the issue's `(*enemyStates.borrow())` generated code).
const SRC: &str = "\
type EnemyState = { timer: i32 }
let enemyStates: EnemyState[] = []
function update() { enemyStates.push({ timer: 0 }) }
update()
console.log(enemyStates.length)
";

#[test]
fn wrapped_struct_push_and_length_do_not_box_the_whole_vec() {
    let rust = gba_emit(SRC, "regr562.tish");
    // No whole-Vec boxing (the E0282-prone untyped `map(|v| …)` closure) for either push or length.
    assert!(
        !rust.contains("iter().cloned().map(|v|"),
        "#562: wrapped struct push/length must NOT box the whole Vec via an untyped `map(|v| …)`\n{rust}"
    );
    assert!(
        !rust.contains("array_push(&Value::Array(VmRef::new"),
        "#562: push must not fall to the boxed `array_push` on a throwaway Value::Array copy\n{rust}"
    );
    // Native push through the shared cell, and a native `len()` for `.length`.
    assert!(
        rust.contains(".borrow_mut()).push("),
        "#562: push must be native `(*enemyStates.borrow_mut()).push(..)`\n{rust}"
    );
    assert!(
        rust.contains("enemyStates.borrow(); __bg.len()"),
        "#562: `.length` must be a native scoped `{{ let __bg = enemyStates.borrow(); __bg.len() }}` (#567), not a whole-Vec box\n{rust}"
    );
}
