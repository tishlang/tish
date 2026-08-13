//! #654 — an immutable captured binding must not be re-read out of a `VmRef` cell on every call.
//!
//! A module-level `const` that is never reassigned was still lowered to a `VmRef` cell and
//! re-borrowed + cloned out of it on EVERY call of any function mentioning it. For a per-frame
//! function that is repeated work to fetch a value fixed at compile time, and the cost scales with
//! how many constants the function happens to name — an invisible relationship that pushed authors
//! toward magic numbers over named constants.
//!
//! A read-only captured var is never assigned anywhere in its defining scope (that is exactly what
//! keeps it out of `rc_cell_storage`), so it is snapshot by value at closure creation instead.

use std::fs;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

#[test]
fn immutable_capture_needs_no_cell() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("hero.tish"),
        "export const HERO_NAME = \"link\"\n",
    )
    .unwrap();
    fs::write(
        root.join("main.tish"),
        "import { HERO_NAME } from \"./hero.tish\"\n\
         fn heroAnim(t) { return HERO_NAME + t }\n\
         fn main() { console.log(heroAnim(1)) }\n\
         main()\n",
    )
    .unwrap();
    let path = root.join("main.tish");
    let rust = compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile")
        .0;

    assert!(
        !rust.contains("HERO_NAME_cell"),
        "an immutable captured const must not get a VmRef cell (#654):\n{rust}"
    );
    assert!(
        rust.contains("let mut HERO_NAME = HERO_NAME_capt.clone();"),
        "the call body must bind from the captured snapshot, not a per-call cell borrow (#654). \
         The `mut` is #669: the binding can be handed to a native-vec fn as `&mut Vec`."
    );
}
