//! tishlang/tish#556 — a top-level MUTABLE var captured by a closure becomes a `VmRef` cell; if its
//! name matches an imported/other function's PARAMETER, a reference to the parameter inside that
//! function must resolve to the parameter, NOT a `vm_read` of the outer cell (which emits
//! `vm_read(&param)` — a `&Value` where a `&VmRef` is wanted, so the generated Rust won't compile).
//! The bug bit the GBA backend; guard both emit modes.
use std::collections::HashSet;
use std::path::PathBuf;
use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn compile(rel: &str, mode: NativeEmitMode) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let caps: HashSet<String> = HashSet::new();
    compile_project_full_emit(&path, path.parent(), &[], true, mode, Some(&caps))
        .unwrap()
        .0
}

#[test]
fn param_shadows_outer_cell_not_read_via_vm_read() {
    let rel = "crates/tish_compile/tests/regression/name_collision_main.tish";
    for mode in [NativeEmitMode::DesktopBin, NativeEmitMode::Gba] {
        let rust = compile(rel, mode);
        // The bug: inside `loadScene`, calling the `scene` PARAMETER (`scene(1)`) compiled the callee
        // as `vm_read(&scene)` — a read of the outer `VmRef` cell, i.e. `&Value` where a `&VmRef` is
        // wanted (won't compile). The param must be used directly instead. (A plain `vm_read(&scene)`
        // still legitimately appears where `bump` mutates the real outer cell, so we target this
        // exact param-as-callee shape.)
        assert!(
            !rust.contains("_callee = (tishlang_runtime::vm_read(&scene))"),
            "#556 ({mode:?}): `loadScene`'s `scene` parameter must shadow the outer cell — a call to \
             it must not read `vm_read(&scene)`\n{rust}"
        );
        // The outer cell itself is still legitimately a real `VmRef` where `bump` mutates it.
        assert!(
            rust.contains("scene.borrow_mut()"),
            "#556 ({mode:?}): expected the outer captured `scene` to remain a real `VmRef` cell\n{rust}"
        );
    }
}
