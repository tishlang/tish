//! #684 — a boxed closure that WRITES an outer module binding must have that binding's cell in
//! scope.
//!
//! The write path emits `*name.borrow_mut() = …` whenever `refcell_wrapped_vars` — a whole-program
//! prepass — says the binding is a capture cell. The binding it refers to comes from
//! `outer_vars_stack`, which is built as statements are emitted. When a closure is created ABOVE
//! the `let`, the two disagree and nothing is in scope:
//!
//! ```text
//! error[E0425]: cannot find value `deadCol` in this scope
//! ```
//!
//! #629's hoist does not reach this shape — it only moves a literal declaration referenced from a
//! top-level `function`, not one reached through a `let` holding an arrow. The cell is now declared
//! up front instead, and the `let` assigns into it.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

const FIXTURE: &str = "../../tests/regression/closure_writes_module_let_684.tish";

/// #682's module statics give the same binding a home that is in scope everywhere, which fixes
/// this shape too. The pre-declaration below is what covers it everywhere the statics are OFF —
/// the kill switch, `RustLib`, a program that can run tish on another thread, and any binding a
/// local elsewhere shadows — so that is the mode these assertions pin. This whole file is its own
/// test binary, so the env var cannot leak into another test's compile.
fn rust_for(mode: NativeEmitMode) -> String {
    std::env::set_var("TISH_MODULE_STATICS", "0");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    compile_project_full_emit(&path, path.parent(), &[], true, mode, None)
        .expect("compile")
        .0
}

fn run_body(rust: &str) -> String {
    rust.split("fn run()")
        .nth(1)
        .expect("generated crate must have a run()")
        .to_string()
}

/// The cell is declared before every statement, and the `let` assigns into it rather than
/// shadowing it with a second `VmRef`.
#[test]
fn module_let_written_from_a_closure_above_it_gets_its_cell_first() {
    for mode in [NativeEmitMode::Gba, NativeEmitMode::DesktopBin] {
        let body = run_body(&rust_for(mode));
        let decl = body
            .find("let deadCol: VmRef<Value> = VmRef::new(Value::Null);")
            .unwrap_or_else(|| {
                panic!("deadCol's cell must be pre-declared for {mode:?}; got:\n{body}")
            });
        let write = body
            .find("*deadCol.borrow_mut()")
            .unwrap_or_else(|| panic!("expected a cell write for {mode:?}; got:\n{body}"));
        assert!(
            decl < write,
            "the cell must be declared before the first `*deadCol.borrow_mut()` — that ordering IS \
             the bug ({mode:?}):\n{body}"
        );
        assert!(
            !body.contains("let deadCol = VmRef::new("),
            "a second `let deadCol = VmRef::new(..)` would leave the closures above it writing a \
             cell nothing reads ({mode:?}):\n{body}"
        );
    }
}

/// A binding with no closure above its declaration keeps the ordinary `let x = VmRef::new(init)`
/// form — the pre-declaration is the residue path, not the default.
#[test]
fn a_binding_with_no_forward_capture_is_untouched() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr684_plain");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.tish");
    std::fs::write(
        &src,
        "export let deadCol: i32 = -1\n\
         function bump(c: i32) { deadCol = c }\n\
         function main() { bump(3); console.log(deadCol) }\n",
    )
    .unwrap();
    std::env::set_var("TISH_MODULE_STATICS", "0");
    let rust = compile_project_full_emit(
        &src,
        Some(dir.as_path()),
        &[],
        true,
        NativeEmitMode::Gba,
        None,
    )
    .expect("compile")
    .0;
    let body = run_body(&rust);
    assert!(
        body.contains("let deadCol = VmRef::new("),
        "no forward capture here, so the plain declaration must survive:\n{body}"
    );
    assert!(
        !body.contains("let deadCol: VmRef<Value> = VmRef::new(Value::Null);"),
        "pre-declaration must not fire without a forward capture:\n{body}"
    );
}
