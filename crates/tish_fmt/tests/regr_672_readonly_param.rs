//! #672 — the `readonly` marker on a `declare fn` parameter must survive formatting.
//!
//! It is not cosmetic: it is what lets an array handed to that native keep its typed representation
//! instead of being boxed on every read (#663 has to assume the worst for an invisible body). A
//! formatter that dropped it would silently reintroduce the 3.8x-per-read cost on the next `fmt`.

#[test]
fn readonly_marker_round_trips() {
    let src = "declare fn sinkRO(w: i32, h: i32, readonly data: i32[]): void\n";
    let out = tishlang_fmt::format_source(src).expect("format");
    assert!(
        out.contains("readonly data: i32[]"),
        "formatting must preserve the `readonly` marker — dropping it silently re-boxes every \
         read of the array passed here (#672). Got:\n{out}"
    );
    let twice = tishlang_fmt::format_source(&out).expect("format twice");
    assert_eq!(out, twice, "formatting must be idempotent");
}

#[test]
fn unmarked_declare_fn_is_unchanged() {
    let src = "declare fn plain(a: i32, b: i32[]): void\n";
    let out = tishlang_fmt::format_source(src).expect("format");
    assert!(
        !out.contains("readonly"),
        "an unmarked declaration must not gain a marker:\n{out}"
    );
}

/// `readonly` is not a reserved word — a parameter may still be named that.
#[test]
fn parameter_named_readonly_still_works() {
    let src = "declare fn named(readonly: i32): void\n";
    let out = tishlang_fmt::format_source(src).expect("format");
    assert!(
        out.contains("named(readonly: i32)"),
        "a parameter literally named `readonly` must round-trip as itself:\n{out}"
    );
}
