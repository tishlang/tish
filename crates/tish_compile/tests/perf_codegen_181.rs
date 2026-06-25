//! #181 — direct Map method dispatch for `new Map()` locals.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn enable_typed_flags() {
}

#[test]
fn k_nucleotide_uses_direct_map_has_get_set() {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/perf/k_nucleotide.tish")
        .canonicalize()
        .unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(
        rust.contains("tish_map_has("),
        "expected direct map_has dispatch:\n{}",
        rust.lines()
            .filter(|l| l.contains("map_has") || l.contains("map_set"))
            .take(6)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        rust.contains("tish_map_get(") && rust.contains("tish_map_set("),
        "expected direct map_get/set dispatch"
    );
    assert!(
        !rust.contains("get_prop(&(m).clone(), \"has\")"),
        "should not use bound-method has on map local m"
    );
}
