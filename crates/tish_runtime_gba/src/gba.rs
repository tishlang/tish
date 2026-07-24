//! GBA runtime entry points, called by the generated `#[agb::entry] agb_main`.
//!
//! Contract (see tish-gba/CONTRACT.md §4):
//! ```ignore
//! #[agb::entry]
//! fn agb_main(gba: agb::Gba) -> ! {
//!     tishlang_runtime::gba::init(gba);
//!     let _ = run();              // P5: block_on(run())
//!     tishlang_runtime::gba::halt()
//! }
//! ```

use alloc::vec::Vec;
use core::cell::RefCell;
use tishlang_core::SingleCore;

// The `agb::Gba` peripheral bundle, stashed at boot so the low-level binding
// crate (`tish-agb`) can take it and drive graphics/input/etc. Single-core, so a
// plain static behind the SingleCore `.with()` API is sound.
static GBA: SingleCore<RefCell<Option<agb::Gba>>> = SingleCore::new(RefCell::new(None));

/// Stash the `agb::Gba` peripheral bundle for the binding crate to claim via
/// [`take_gba`]. Called by the generated `agb_main` before the program body runs.
pub fn init(gba: agb::Gba) {
    GBA.with(|c| {
        *c.borrow_mut() = Some(gba);
    });
}

/// Claim the `agb::Gba` peripheral bundle (once). `tish-agb` calls this on first
/// use to set up its graphics/input context. Returns `None` if already taken.
pub fn take_gba() -> Option<agb::Gba> {
    GBA.with(|c| c.borrow_mut().take())
}

/// Divergent halt for the end of `agb_main` (after `run()` returns).
pub fn halt() -> ! {
    loop {
        agb::halt();
    }
}

// ── Per-frame hooks ──────────────────────────────────────────────────────────
// `tish-agb`'s frame driver and (later) the engine pipeline register here; the
// frame loop runs them once per frame before `commit()`. Single-core, so a plain
// static with the SingleCore `.with()` API.
static PRE_COMMIT: SingleCore<RefCell<Vec<fn()>>> = SingleCore::new(RefCell::new(Vec::new()));

/// Register a callback to run once per frame, before the display commit.
pub fn register_pre_commit(f: fn()) {
    PRE_COMMIT.with(|c| c.borrow_mut().push(f));
}

/// Run all registered pre-commit hooks (called by the frame driver / executor).
pub fn run_pre_commit() {
    PRE_COMMIT.with(|c| {
        // Clone out the fn pointers so a hook can register another without a
        // nested borrow of the same RefCell.
        let hooks: Vec<fn()> = c.borrow().clone();
        for f in hooks {
            f();
        }
    });
}

// ── Asset registry ───────────────────────────────────────────────────────────
// Sprite sheets from `asset:` imports. The generated `agb_main` calls
// `__asset_register_sheet` once per imported asset (in import order) BEFORE
// `run()`, handing over the `&'static [Sprite]` slice that agb's
// `include_aseprite_inner!` produced in the generated crate. The returned i32 is
// the handle a tish program passes to `tish-agb`'s sprite APIs; `tish-agb` reads
// the slice back with [`asset_sheet`]. The registry lives here (not in tish-agb)
// so the generated crate — which always depends on this facade as
// `tishlang_runtime` — can register without needing the `cargo:tish_agb` dep.

/// One registered sprite sheet: the frames of an `asset:` import.
static ASSET_SHEETS: SingleCore<RefCell<Vec<&'static [agb::display::object::Sprite]>>> =
    SingleCore::new(RefCell::new(Vec::new()));

/// Register a sprite sheet, returning its i32 handle (= its registration order).
/// Called by the generated `agb_main`, in import order, before the program body.
pub fn __asset_register_sheet(sheet: &'static [agb::display::object::Sprite]) -> i32 {
    ASSET_SHEETS.with(|c| {
        let mut v = c.borrow_mut();
        let idx = v.len() as i32;
        v.push(sheet);
        idx
    })
}

/// Look up a registered sprite sheet by handle. `None` if out of range.
pub fn asset_sheet(handle: i32) -> Option<&'static [agb::display::object::Sprite]> {
    ASSET_SHEETS.with(|c| c.borrow().get(handle as usize).copied())
}

/// One registered background: its palettes + full-screen tile data (agb
/// `include_background_gfx!` output). Registered by the generated `agb_main` for
/// each `background:` import; `tish-agb`'s `bg_new` builds a `RegularBackground`.
type BgAsset = (
    &'static [agb::display::Palette16],
    &'static agb::display::tile_data::TileData,
);

static ASSET_BGS: SingleCore<RefCell<Vec<BgAsset>>> = SingleCore::new(RefCell::new(Vec::new()));

/// Register a background (palettes + tile data), returning its i32 handle.
pub fn __asset_register_bg(bg: BgAsset) -> i32 {
    ASSET_BGS.with(|c| {
        let mut v = c.borrow_mut();
        let idx = v.len() as i32;
        v.push(bg);
        idx
    })
}

/// Look up a registered background by handle. `None` if out of range.
pub fn asset_bg(handle: i32) -> Option<BgAsset> {
    ASSET_BGS.with(|c| c.borrow().get(handle as usize).copied())
}

/// Registered sounds (`wav:` imports). `SoundData` is `Copy` (a `&'static [u8]` handle).
static ASSET_WAVS: SingleCore<RefCell<Vec<agb::sound::mixer::SoundData>>> =
    SingleCore::new(RefCell::new(Vec::new()));

/// Register a sound, returning its i32 handle.
pub fn __asset_register_wav(data: agb::sound::mixer::SoundData) -> i32 {
    ASSET_WAVS.with(|c| {
        let mut v = c.borrow_mut();
        let idx = v.len() as i32;
        v.push(data);
        idx
    })
}

/// Look up a registered sound by handle. `None` if out of range.
pub fn asset_wav(handle: i32) -> Option<agb::sound::mixer::SoundData> {
    ASSET_WAVS.with(|c| c.borrow().get(handle as usize).copied())
}
