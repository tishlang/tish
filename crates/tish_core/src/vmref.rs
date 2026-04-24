//! Shared-mutable reference used by the Tish runtime for `Value::Array`,
//! `Value::Object`, and `Value::RegExp` payloads.
//!
//! ## Why this exists
//!
//! Tish's `Value` uses interior mutability for arrays, objects, and regex
//! state. Historically that was `Rc<RefCell<T>>`, which is fast but
//! `!Send` — so `Value` couldn't move across threads, which in turn meant
//! `serve(port, handler)` had to serialise every request through one
//! VM dispatcher thread.
//!
//! `VmRef<T>` lets the build system pick the right trade-off **per
//! compile target**:
//!
//! | feature `send-values`     | `VmRef<T>`              | `NativeFn`                       | targets                                          |
//! |---------------------------|-------------------------|----------------------------------|--------------------------------------------------|
//! | **off** *(default)*       | `Rc<RefCell<T>>`        | `Rc<dyn Fn + 'static>`           | wasm32, wasi, interpreter, cranelift/llvm VMs    |
//! | **on**                    | `Arc<Mutex<T>>`         | `Arc<dyn Fn + Send + Sync>`      | Rust native with `http` enabled (server workloads) |
//!
//! The *API* is identical in both configurations (`borrow` / `borrow_mut`
//! / `ptr_eq` / `Clone`), so every existing call site in the workspace
//! compiles unchanged. What flips is only the underlying primitive.
//!
//! ## Why this matters for performance
//!
//! * **wasm / wasi / cranelift / llvm / interpreter**: still pure
//!   `Rc<RefCell<T>>`. Zero atomic ops, no mutex churn, behaviour
//!   bit-identical to the pre-migration baseline.
//! * **Rust native, non-server**: same — `send-values` only activates
//!   when something in the dependency graph (usually `http`) needs it.
//! * **Rust native with server**: `Arc<Mutex<T>>` pays ~3–5 ns per
//!   `borrow` in the uncontended case (single atomic CAS). On Tish's
//!   hot paths — roughly 6–12 borrows per request — that's ~30–60 ns of
//!   overhead. In exchange we get `N×` handler scaling across cores,
//!   which recovers orders of magnitude more throughput than it costs.
//!
//! ## API surface
//!
//! ```ignore
//! let cell = VmRef::new(42);
//! *cell.borrow() + 1;          // read
//! *cell.borrow_mut() = 99;     // write
//! VmRef::ptr_eq(&a, &b);       // identity
//! let clone = cell.clone();    // shared ownership
//! ```
//!
//! Returned guard types (`VmReadGuard<'_, T>`, `VmWriteGuard<'_, T>`) are
//! type aliases that pick `Ref`/`RefMut` or `MutexGuard` depending on the
//! feature. They both `Deref` (and, for write guards, `DerefMut`) to `T`
//! just like the underlying types.

use std::fmt;

// --------------------------------------------------------------------------
// Single-threaded backing store (default): Rc<RefCell<T>>
// --------------------------------------------------------------------------
#[cfg(not(feature = "send-values"))]
mod imp {
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    pub struct VmRef<T: ?Sized>(pub(super) Rc<RefCell<T>>);

    /// Read guard alias. On the single-threaded path this is a true
    /// `Ref<'_, T>`, so multiple readers can coexist.
    pub type ReadGuard<'a, T> = std::cell::Ref<'a, T>;
    /// Write guard alias. Exclusive, `DerefMut`.
    pub type WriteGuard<'a, T> = std::cell::RefMut<'a, T>;

    impl<T> VmRef<T> {
        #[inline]
        pub fn new(value: T) -> Self {
            VmRef(Rc::new(RefCell::new(value)))
        }
    }

    impl<T: ?Sized> VmRef<T> {
        #[inline]
        pub fn borrow(&self) -> ReadGuard<'_, T> {
            self.0.borrow()
        }

        #[inline]
        pub fn borrow_mut(&self) -> WriteGuard<'_, T> {
            self.0.borrow_mut()
        }

        #[inline]
        pub fn ptr_eq(a: &Self, b: &Self) -> bool {
            Rc::ptr_eq(&a.0, &b.0)
        }

        #[inline]
        pub fn strong_count(this: &Self) -> usize {
            Rc::strong_count(&this.0)
        }
    }

    impl<T: ?Sized> Clone for VmRef<T> {
        #[inline]
        fn clone(&self) -> Self {
            VmRef(Rc::clone(&self.0))
        }
    }
}

// --------------------------------------------------------------------------
// Thread-safe backing store (opt-in): Arc<Mutex<T>>
// --------------------------------------------------------------------------
#[cfg(feature = "send-values")]
mod imp {
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct VmRef<T: ?Sized>(pub(super) Arc<Mutex<T>>);

    /// Read guard alias. On the multi-threaded path both readers and
    /// writers share a single `MutexGuard` (exclusive access).
    pub type ReadGuard<'a, T> = std::sync::MutexGuard<'a, T>;
    /// Write guard alias.
    pub type WriteGuard<'a, T> = std::sync::MutexGuard<'a, T>;

    impl<T> VmRef<T> {
        #[inline]
        pub fn new(value: T) -> Self {
            VmRef(Arc::new(Mutex::new(value)))
        }
    }

    impl<T: ?Sized> VmRef<T> {
        /// Acquire the inner mutex. Poisoning is swallowed — a Tish
        /// handler panic already aborts the enclosing thread; there is
        /// no invariant worth preserving past that point.
        #[inline]
        pub fn borrow(&self) -> ReadGuard<'_, T> {
            self.0.lock().unwrap_or_else(|p| p.into_inner())
        }

        #[inline]
        pub fn borrow_mut(&self) -> WriteGuard<'_, T> {
            self.0.lock().unwrap_or_else(|p| p.into_inner())
        }

        #[inline]
        pub fn ptr_eq(a: &Self, b: &Self) -> bool {
            Arc::ptr_eq(&a.0, &b.0)
        }

        #[inline]
        pub fn strong_count(this: &Self) -> usize {
            Arc::strong_count(&this.0)
        }
    }

    impl<T: ?Sized> Clone for VmRef<T> {
        #[inline]
        fn clone(&self) -> Self {
            VmRef(Arc::clone(&self.0))
        }
    }
}

pub use imp::{ReadGuard as VmReadGuard, VmRef, WriteGuard as VmWriteGuard};

impl<T: fmt::Debug> fmt::Debug for VmRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Match `RefCell`'s debug format so snapshot-test output stays
        // stable across the migration.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = self.borrow();
            format!("{:?}", &*guard)
        })) {
            Ok(s) => write!(f, "RefCell {{ value: {} }}", s),
            Err(_) => write!(f, "RefCell {{ value: <borrowed> }}"),
        }
    }
}
