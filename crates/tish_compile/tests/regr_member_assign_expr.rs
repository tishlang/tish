//! Regression: a native-struct field assignment in EXPRESSION position (`let y = (p.x = 5)`,
//! chained `a.x = b.y = v`, `if ((p.x = v) > 0)`) must evaluate to the assigned value, not
//! `Value::Null`. The fast path (`try_emit_native_member_assign`) now boxes and returns the stored
//! value: `{ let _v = …; let _r = <boxed>; p.x = _v; _r }`.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

#[test]
fn expr_position_native_member_assign_yields_value_not_null() {
    let rust = compile("tests/regression/member_assign_expr.tish");
    // The fixture's only member assignment is in expression position, so the value-returning form
    // (`let _r = …; <lhs> = _v; _r`) must appear. Before the fix the assignment emitted `Value::Null`
    // as its result and there was no `_r` binding.
    assert!(
        rust.contains("let _r ="),
        "expression-position native member assign must box + yield the assigned value \
         (found no `let _r =` value-return form):\n{}",
        rust
    );
}
