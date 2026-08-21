//! #684 with #682's module statics ON (the default): the binding has a home that is in scope
//! everywhere, so the closure above its `let` has nothing to capture and nothing to get wrong.
//!
//! Its own test binary because its sibling file pins the statics-OFF emission with a process-wide
//! env var.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

#[test]
fn the_write_resolves_through_the_static() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/regression/closure_writes_module_let_684.tish");
    for mode in [NativeEmitMode::Gba, NativeEmitMode::DesktopBin] {
        let rust = compile_project_full_emit(&path, path.parent(), &[], true, mode, None)
            .expect("compile")
            .0;
        assert!(
            rust.contains("fn __tish_gv_deadCol()"),
            "`deadCol` should have a static home ({mode:?}):\n{rust}"
        );
        let body = rust.split("fn run()").nth(1).expect("run()");
        let handle = body
            .find("let deadCol = __tish_gv_deadCol();")
            .unwrap_or_else(|| panic!("expected the handle binding ({mode:?}):\n{body}"));
        let write = body
            .find("*deadCol.borrow_mut()")
            .unwrap_or_else(|| panic!("expected the cell write ({mode:?}):\n{body}"));
        assert!(
            handle < write,
            "the handle must be bound before the first write — that ordering IS #684 ({mode:?}):\n{body}"
        );
    }
}
