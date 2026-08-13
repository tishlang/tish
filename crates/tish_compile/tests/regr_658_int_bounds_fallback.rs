//! #658 — the BOUNDS-CHECKED read of a promoted integer array must stay in the integer domain.
//!
//! #645 gave the masked/provably-in-range read a direct `G_LIT[i]` load. The fallback emitted for
//! every other index shape stayed `f64`, so a merely-computed index paid two soft-float calls per
//! element on a chip with no FPU — `G_LIT[_i] as f64` on the way out and `as i32` at the consumer.
//! The report measures ~25 ticks/element against ~0.4 for the identical read behind a mask.
//!
//! The `f64::NAN` sentinel that shape existed to preserve is not a semantic any backend agrees
//! with: interp/vm/node all answer `null` for an out-of-range read, and only native answered `NaN`.
//! An integer consumer cannot tell the difference either way, since `f64::NAN as i32` is already 0.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn gba_rust(fixture: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(fixture);
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile for gba")
        .0
}

#[test]
fn computed_index_read_has_no_f64_round_trip() {
    let rust = gba_rust("fixtures_658_int_index.tish");
    let lit = rust
        .lines()
        .filter(|l| l.contains("G_LIT[_i]"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !lit.is_empty(),
        "expected a bounds-checked read of the promoted array:\n{rust}"
    );
    assert!(
        !lit.contains("G_LIT[_i] as f64"),
        "the bounds-checked read must not widen the element to f64 — on ARM7TDMI that is a \
         soft-float call per element, and the `as i32` at the consumer is a second one (#658):\n{lit}"
    );
    assert!(
        !lit.contains("f64::NAN"),
        "an integer array's fallback must use an integer sentinel, not NaN — the NaN branch is \
         what forced the whole expression back into the f64 domain (#658):\n{lit}"
    );
}

/// The proven-in-range form (#645) must still be the bare load — this must not regress it.
#[test]
fn masked_index_read_stays_a_direct_load() {
    let rust = gba_rust("fixtures_658_int_index.tish");
    assert!(
        rust.contains("G_LIT[((("),
        "the masked index must remain a direct integer-domain load (#645):\n{rust}"
    );
}
