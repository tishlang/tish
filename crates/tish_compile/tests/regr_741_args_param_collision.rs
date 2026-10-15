//! tishlang/tish#741 — the native backend emits each function as
//! `Value::native(move |args: &[Value]| { … })` and binds every parameter out of that
//! slice. A parameter literally NAMED `args` shadows the slice, so each parameter bound
//! after it read `.get(n)` off a `Value` — `error[E0599]: no method named `get` found for
//! enum `Value``. The interpreter and the JS target accept the same source, so a shared
//! module could carry this until its first native build.
//!
//! The prologue now reads through `__tish_args` whenever a parameter shadows the slice,
//! and is left exactly as it was when nothing does.
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

const REL: &str = "crates/tish_compile/tests/regression/args_param_collision_main.tish";

#[test]
fn param_named_args_does_not_shadow_the_slice() {
    for mode in [NativeEmitMode::DesktopBin, NativeEmitMode::Gba] {
        let rust = compile(REL, mode);

        // The alias is bound from the real slice before anything can shadow it.
        assert!(
            rust.contains("let __tish_args = args;"),
            "#741 ({mode:?}): expected a slice alias for the functions whose parameter is named `args`\n{rust}"
        );

        // `collides(args, cwd)`: BOTH parameters come off the alias. Before the fix the
        // second one read `args.get(1)` — off the `Value` the first one had just bound.
        assert!(
            rust.contains("let mut args = __tish_args.get(0)"),
            "#741 ({mode:?}): the `args` parameter itself must bind from the alias\n{rust}"
        );
        assert!(
            rust.contains("__tish_args.get(1)"),
            "#741 ({mode:?}): the parameter after `args` must read the alias, not the shadowed value\n{rust}"
        );

        // `middle(a, args, c)`: the collision is positional, so `c` (index 2) must use the
        // alias too — proving the guard is per-function, not "is `args` first".
        assert!(
            rust.contains("__tish_args.get(2)"),
            "#741 ({mode:?}): a parameter after a mid-list `args` must read the alias\n{rust}"
        );

        // No parameter may be bound off the bare slice in a function that aliases it: any
        // such read is the shadowed `Value` again.
        for shape in [
            "let mut cwd = args.get(",
            "let mut c = args.get(",
        ] {
            assert!(
                !rust.contains(shape),
                "#741 ({mode:?}): `{shape}…` reads the shadowed value, not the slice\n{rust}"
            );
        }
    }
}

#[test]
fn functions_without_the_collision_are_unchanged() {
    for mode in [NativeEmitMode::DesktopBin, NativeEmitMode::Gba] {
        let rust = compile(REL, mode);
        // `plain(items, cwd)` has no parameter named `args`, so its prologue must still
        // read the slice directly — the fix must not churn ordinary codegen.
        assert!(
            rust.contains("let mut items = args.get(0)"),
            "#741 ({mode:?}): a function with no collision must keep reading `args` directly\n{rust}"
        );
    }
}
