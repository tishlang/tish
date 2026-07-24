//! Portability layer for the `portable` (no_std + alloc) build.
//!
//! Several items here (the lock/once shims, the clock/RNG hooks) are consumed by
//! `tishlang_builtins` and the GBA facade rather than by `tishlang_core` itself,
//! so within this crate they read as unused.
#![allow(dead_code, unused_imports)]

//!
//! Every use-site that would otherwise reach for a std-only or atomics-bearing
//! primitive imports it from here instead (`crate::compat::{Arc, ArcStr,
//! AHashMap, RandomState, AtomicU64, ...}`), so the *same source* compiles for
//! the host and for `thumbv4t-none-eabi` (Game Boy Advance).
//!
//! - **default (std)**: transparent re-exports of `std::sync::Arc`, `ahash`,
//!   `arcstr`, `std` atomics — behaviour is byte-for-byte the pre-split build.
//! - **`portable`**: atomics-free, alloc-based equivalents — `Rc` for `Arc`,
//!   `FxHasher` for `ahash` (foldhash/ahash both need atomics; see P0 findings),
//!   `portable-atomic` for the one `AtomicU64`, an `Rc<str>` `ArcStr`, and a
//!   `FloatExt` trait supplying the f64 transcendentals `core` lacks.
//!
//! GBA is single-core with cooperative scheduling and no preemptive interrupts
//! touching interpreter state, so the process-global singletons that are
//! `thread_local!` on the host become single-threaded statics here.

// ===========================================================================
// Standard (hosted) backing — transparent re-exports.
// ===========================================================================
#[cfg(not(feature = "portable"))]
pub use std_backing::*;

#[cfg(not(feature = "portable"))]
mod std_backing {
    pub use ahash::{AHashMap, RandomState};
    pub use arcstr::ArcStr;
    pub use std::sync::atomic::{AtomicU64, Ordering};
    pub use std::sync::{Arc, Mutex, OnceLock, RwLock};
}

// ===========================================================================
// Portable (no_std + alloc) backing.
// ===========================================================================
#[cfg(feature = "portable")]
pub use portable_backing::*;

#[cfg(feature = "portable")]
mod portable_backing {
    use alloc::rc::Rc;

    /// Single-core target: reference counting needs no atomics.
    pub use alloc::rc::Rc as Arc;

    /// 64-bit atomic via `portable-atomic` (critical-section backed on GBA;
    /// the impl is supplied at final link by agb). Only used for the symbol-id
    /// counter, which on a single core is really just a `Cell`, but keeping the
    /// atomic type keeps the source identical to the std path.
    pub use portable_atomic::{AtomicU64, Ordering};

    /// FxHasher — deterministic, atomics-free, and exactly what `agb_hashmap`
    /// uses on GBA. Drop-in for `ahash::RandomState`.
    pub type RandomState = rustc_hash::FxBuildHasher;

    /// Drop-in for `ahash::AHashMap<K, V>` (same `default()` / `get` / `insert`
    /// / `iter` / `entry` surface), hashed with FxHasher.
    pub type AHashMap<K, V> = hashbrown::HashMap<K, V, RandomState>;

    // -- ArcStr: an immutable, cheaply-cloneable string. On the host this is
    //    `arcstr::ArcStr` (thin, atomic-refcounted); here it wraps `Rc<str>`.
    //    `Rc<str>` is a 16-byte fat pointer, which still fits the `Value`
    //    24-byte budget (the `Arc<dyn ...>` variants already set that width).
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ArcStr(Rc<str>);

    impl ArcStr {
        #[inline]
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }

    impl Default for ArcStr {
        #[inline]
        fn default() -> Self {
            ArcStr(Rc::from(""))
        }
    }

    impl core::ops::Deref for ArcStr {
        type Target = str;
        #[inline]
        fn deref(&self) -> &str {
            &self.0
        }
    }

    impl core::convert::AsRef<str> for ArcStr {
        #[inline]
        fn as_ref(&self) -> &str {
            &self.0
        }
    }

    impl core::borrow::Borrow<str> for ArcStr {
        #[inline]
        fn borrow(&self) -> &str {
            &self.0
        }
    }

    impl From<&str> for ArcStr {
        #[inline]
        fn from(s: &str) -> Self {
            ArcStr(Rc::from(s))
        }
    }

    impl From<alloc::string::String> for ArcStr {
        #[inline]
        fn from(s: alloc::string::String) -> Self {
            ArcStr(Rc::from(s.as_str()))
        }
    }

    impl From<&alloc::string::String> for ArcStr {
        #[inline]
        fn from(s: &alloc::string::String) -> Self {
            ArcStr(Rc::from(s.as_str()))
        }
    }

    impl From<Rc<str>> for ArcStr {
        #[inline]
        fn from(s: Rc<str>) -> Self {
            ArcStr(s)
        }
    }

    impl core::fmt::Display for ArcStr {
        #[inline]
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl core::fmt::Debug for ArcStr {
        #[inline]
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            core::fmt::Debug::fmt(self.as_str(), f)
        }
    }

    impl PartialEq<str> for ArcStr {
        #[inline]
        fn eq(&self, other: &str) -> bool {
            self.as_str() == other
        }
    }

    impl PartialEq<&str> for ArcStr {
        #[inline]
        fn eq(&self, other: &&str) -> bool {
            self.as_str() == *other
        }
    }

    // -- Single-core lock/once shims. GBA runs one cooperative thread, so these
    //    never actually contend; they exist only to keep the std API surface
    //    (`.read()`/`.write()`/`.lock()`/`.get_or_init()`) at call sites.
    use core::cell::{Ref, RefCell, RefMut, UnsafeCell};

    /// Never-poisoned, never-contended stand-in for `std::sync::RwLock`.
    pub struct RwLock<T>(RefCell<T>);
    impl<T> RwLock<T> {
        #[inline]
        pub const fn new(v: T) -> Self {
            RwLock(RefCell::new(v))
        }
        #[inline]
        pub fn read(&self) -> Result<Ref<'_, T>, core::convert::Infallible> {
            Ok(self.0.borrow())
        }
        #[inline]
        pub fn write(&self) -> Result<RefMut<'_, T>, core::convert::Infallible> {
            Ok(self.0.borrow_mut())
        }
    }

    /// Stand-in for `std::sync::Mutex` (`.lock()` yields a mut guard).
    pub struct Mutex<T>(RefCell<T>);
    impl<T> Mutex<T> {
        #[inline]
        pub const fn new(v: T) -> Self {
            Mutex(RefCell::new(v))
        }
        #[inline]
        pub fn lock(&self) -> Result<RefMut<'_, T>, core::convert::Infallible> {
            Ok(self.0.borrow_mut())
        }
    }

    /// Stand-in for `std::sync::OnceLock` (`new`/`get`/`get_or_init`/`set`).
    pub struct OnceLock<T>(UnsafeCell<Option<T>>);
    // SAFETY: single-core, cooperative; no concurrent access (see SingleCore).
    unsafe impl<T> Sync for OnceLock<T> {}
    impl<T> OnceLock<T> {
        #[inline]
        pub const fn new() -> Self {
            OnceLock(UnsafeCell::new(None))
        }
        #[inline]
        pub fn get(&self) -> Option<&T> {
            // SAFETY: single-core; the &mut in get_or_init/set never overlaps a live &.
            unsafe { &*self.0.get() }.as_ref()
        }
        #[inline]
        pub fn set(&self, value: T) -> Result<(), T> {
            // SAFETY: see above.
            let slot = unsafe { &mut *self.0.get() };
            if slot.is_some() {
                return Err(value);
            }
            *slot = Some(value);
            Ok(())
        }
        #[inline]
        pub fn get_or_init<F: FnOnce() -> T>(&self, f: F) -> &T {
            // SAFETY: see above.
            let slot = unsafe { &mut *self.0.get() };
            if slot.is_none() {
                *slot = Some(f());
            }
            slot.as_ref().unwrap()
        }
    }
    impl<T> Default for OnceLock<T> {
        #[inline]
        fn default() -> Self {
            Self::new()
        }
    }
}

// ===========================================================================
// f64 transcendentals — `core` lacks them; libm supplies them under `portable`.
//
// A trait (not free fns) so the SAME `n.abs()` / `n.floor()` source compiles
// both ways: on the host the inherent `f64` method wins; under `portable` (no
// inherent methods exist) the trait method is used. Import it only under
// `portable` at each float use-site to avoid an unused-import warning on host.
// ===========================================================================
#[cfg(feature = "portable")]
pub trait FloatExt {
    fn abs(self) -> Self;
    fn floor(self) -> Self;
    fn ceil(self) -> Self;
    fn trunc(self) -> Self;
    fn fract(self) -> Self;
    fn sqrt(self) -> Self;
    fn cbrt(self) -> Self;
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn tan(self) -> Self;
    fn asin(self) -> Self;
    fn acos(self) -> Self;
    fn atan(self) -> Self;
    fn atan2(self, other: Self) -> Self;
    fn ln(self) -> Self;
    fn log10(self) -> Self;
    fn log2(self) -> Self;
    fn exp(self) -> Self;
    fn exp_m1(self) -> Self;
    fn ln_1p(self) -> Self;
    fn powf(self, n: Self) -> Self;
    fn powi(self, n: i32) -> Self;
    fn rem_euclid(self, m: Self) -> Self;
    fn hypot(self, other: Self) -> Self;
    fn round_ties_even(self) -> Self;
    fn sinh(self) -> Self;
    fn cosh(self) -> Self;
    fn tanh(self) -> Self;
    fn asinh(self) -> Self;
    fn acosh(self) -> Self;
    fn atanh(self) -> Self;
}

#[cfg(feature = "portable")]
impl FloatExt for f64 {
    #[inline]
    fn abs(self) -> f64 {
        libm::fabs(self)
    }
    #[inline]
    fn floor(self) -> f64 {
        libm::floor(self)
    }
    #[inline]
    fn ceil(self) -> f64 {
        libm::ceil(self)
    }
    #[inline]
    fn trunc(self) -> f64 {
        libm::trunc(self)
    }
    #[inline]
    fn fract(self) -> f64 {
        self - libm::trunc(self)
    }
    #[inline]
    fn sqrt(self) -> f64 {
        libm::sqrt(self)
    }
    #[inline]
    fn cbrt(self) -> f64 {
        libm::cbrt(self)
    }
    #[inline]
    fn sin(self) -> f64 {
        libm::sin(self)
    }
    #[inline]
    fn cos(self) -> f64 {
        libm::cos(self)
    }
    #[inline]
    fn tan(self) -> f64 {
        libm::tan(self)
    }
    #[inline]
    fn asin(self) -> f64 {
        libm::asin(self)
    }
    #[inline]
    fn acos(self) -> f64 {
        libm::acos(self)
    }
    #[inline]
    fn atan(self) -> f64 {
        libm::atan(self)
    }
    #[inline]
    fn atan2(self, other: f64) -> f64 {
        libm::atan2(self, other)
    }
    #[inline]
    fn ln(self) -> f64 {
        libm::log(self)
    }
    #[inline]
    fn log10(self) -> f64 {
        libm::log10(self)
    }
    #[inline]
    fn log2(self) -> f64 {
        libm::log2(self)
    }
    #[inline]
    fn exp(self) -> f64 {
        libm::exp(self)
    }
    #[inline]
    fn exp_m1(self) -> f64 {
        libm::expm1(self)
    }
    #[inline]
    fn ln_1p(self) -> f64 {
        libm::log1p(self)
    }
    #[inline]
    fn powf(self, n: f64) -> f64 {
        libm::pow(self, n)
    }
    #[inline]
    fn powi(self, n: i32) -> f64 {
        libm::pow(self, n as f64)
    }
    #[inline]
    fn rem_euclid(self, m: f64) -> f64 {
        let r = libm::fmod(self, m);
        if r < 0.0 {
            r + libm::fabs(m)
        } else {
            r
        }
    }
    #[inline]
    fn hypot(self, other: f64) -> f64 {
        libm::hypot(self, other)
    }
    #[inline]
    fn round_ties_even(self) -> f64 {
        // Round to nearest, ties to even — matches std's `f64::round_ties_even`.
        libm::rint(self)
    }
    #[inline]
    fn sinh(self) -> f64 {
        libm::sinh(self)
    }
    #[inline]
    fn cosh(self) -> f64 {
        libm::cosh(self)
    }
    #[inline]
    fn tanh(self) -> f64 {
        libm::tanh(self)
    }
    #[inline]
    fn asinh(self) -> f64 {
        libm::asinh(self)
    }
    #[inline]
    fn acosh(self) -> f64 {
        libm::acosh(self)
    }
    #[inline]
    fn atanh(self) -> f64 {
        libm::atanh(self)
    }
}

// ===========================================================================
// Single-core "thread local": a plain static behind a `.with()` API matching
// `std::thread::LocalKey`, so declaration sites cfg-swap but call sites don't.
// Sound because GBA runs one cooperative thread and no interrupt handler
// touches interpreter state.
// ===========================================================================
#[cfg(feature = "portable")]
pub struct SingleCore<T>(core::cell::UnsafeCell<T>);

#[cfg(feature = "portable")]
// SAFETY: single-core, cooperative, no interrupt handler reenters interpreter
// state. Access is never concurrent.
unsafe impl<T> Sync for SingleCore<T> {}

#[cfg(feature = "portable")]
impl<T> SingleCore<T> {
    pub const fn new(value: T) -> Self {
        SingleCore(core::cell::UnsafeCell::new(value))
    }

    /// Mirrors `LocalKey::with`: hand out a shared ref to the inner value.
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        // SAFETY: see the `unsafe impl Sync` note above.
        f(unsafe { &*self.0.get() })
    }
}

// ===========================================================================
// Injectable host hooks (clock + RNG). On the host these are unused (std
// provides SystemTime / rand); under `portable` the facade (`tishlang_runtime_gba`)
// installs real sources — agb's timers for the clock, agb's RNG for randomness —
// and until it does they degrade to deterministic defaults instead of panicking.
// ===========================================================================
#[cfg(feature = "portable")]
mod hooks {
    use super::SingleCore;
    use core::cell::Cell;

    static NOW_MS: SingleCore<Cell<fn() -> f64>> = SingleCore::new(Cell::new(default_now_ms));
    static RNG_U64: SingleCore<Cell<fn() -> u64>> = SingleCore::new(Cell::new(default_rng));
    static RNG_STATE: SingleCore<Cell<u64>> = SingleCore::new(Cell::new(0x2545_F491_4F6C_DD1D));

    // Cheap deterministic fallbacks until the facade (tishlang_runtime_gba)
    // installs agb-backed sources.
    fn default_now_ms() -> f64 {
        0.0
    }

    fn default_rng() -> u64 {
        // xorshift64* — deterministic PRNG for a target with no entropy source.
        RNG_STATE.with(|c| {
            let mut x = c.get();
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            c.set(x);
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        })
    }

    /// Facade hook: install a real millisecond clock (e.g. agb timer derived).
    pub fn install_clock(f: fn() -> f64) {
        NOW_MS.with(|c| c.set(f));
    }

    /// Facade hook: install a real RNG source (e.g. `agb::rng`).
    pub fn install_rng(f: fn() -> u64) {
        RNG_U64.with(|c| c.set(f));
    }

    /// Facade hook: reseed the fallback xorshift PRNG.
    pub fn seed_rng(seed: u64) {
        RNG_STATE.with(|c| c.set(seed | 1));
    }

    /// Milliseconds since an arbitrary epoch, for `Date.now()`.
    pub fn now_ms() -> f64 {
        NOW_MS.with(|c| (c.get())())
    }

    /// A raw random `u64`.
    pub fn next_u64() -> u64 {
        RNG_U64.with(|c| (c.get())())
    }

    /// A uniform `f64` in [0, 1), for `Math.random()`.
    pub fn random_f64() -> f64 {
        // 53 mantissa bits → [0, 1).
        (next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[cfg(feature = "portable")]
pub use hooks::{install_clock, install_rng, next_u64, now_ms, random_f64, seed_rng};
