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
//! | **on**                    | `Arc<RwLock<T>>`        | `Arc<dyn Fn + Send + Sync>`      | Rust native with `http` enabled (server workloads) |
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
//! * **Rust native with server**: `Arc<RwLock<T>>` pays ~3–5 ns per
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
//! type aliases that pick `Ref`/`RefMut` or `RwLock` read/write guards depending on the
//! feature. They both `Deref` (and, for write guards, `DerefMut`) to `T`
//! just like the underlying types.

use core::fmt;

// --------------------------------------------------------------------------
// Single-threaded backing store (default): Rc<RefCell<T>>
// --------------------------------------------------------------------------
#[cfg(not(feature = "send-values"))]
mod imp {
    use alloc::rc::Rc;
    use core::cell::RefCell;

    #[derive(Default)]
    pub struct VmRef<T: ?Sized>(pub(super) Rc<RefCell<T>>);

    /// Read guard alias. On the single-threaded path this is a true
    /// `Ref<'_, T>`, so multiple readers can coexist.
    pub type ReadGuard<'a, T> = core::cell::Ref<'a, T>;
    /// Write guard alias. Exclusive, `DerefMut`.
    pub type WriteGuard<'a, T> = core::cell::RefMut<'a, T>;

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

        /// A stable identity pointer for this cell, for cycle detection (#381) — two `VmRef`s share it
        /// iff `ptr_eq`. Not for dereference; only for identity comparison / hashing.
        #[inline]
        pub fn as_ptr(&self) -> *const () {
            Rc::as_ptr(&self.0) as *const ()
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
// Thread-safe backing store (opt-in): Arc<RwLock<T>>
// --------------------------------------------------------------------------
#[cfg(feature = "send-values")]
mod imp {
    use parking_lot::RwLock;
    use std::sync::Arc;

    #[derive(Default)]
    pub struct VmRef<T: ?Sized>(pub(super) Arc<RwLock<T>>);

    /// Read guard alias. Readers share the lock, like `RefCell::borrow`: native codegen reads the
    /// same cell more than once within one statement (`{ a: x, b: n - x }` → two
    /// `(*x.borrow())` temporaries alive until the statement ends), which an exclusive lock turned
    /// into a self-deadlock.
    pub type ReadGuard<'a, T> = parking_lot::RwLockReadGuard<'a, T>;
    /// Write guard alias (exclusive).
    pub type WriteGuard<'a, T> = parking_lot::RwLockWriteGuard<'a, T>;

    impl<T> VmRef<T> {
        #[inline]
        pub fn new(value: T) -> Self {
            VmRef(Arc::new(RwLock::new(value)))
        }
    }

    impl<T: ?Sized> VmRef<T> {
        /// Shared access. `read_recursive` so a thread that already holds a read guard can take
        /// another even while a writer on another thread waits (a plain `read` would queue behind
        /// that writer and deadlock). `parking_lot` rather than `std`: its uncontended lock is a
        /// single atomic, and every property/element access takes one.
        #[inline]
        pub fn borrow(&self) -> ReadGuard<'_, T> {
            self.0.read_recursive()
        }

        /// Exclusive access.
        #[inline]
        pub fn borrow_mut(&self) -> WriteGuard<'_, T> {
            self.0.write()
        }

        #[inline]
        pub fn ptr_eq(a: &Self, b: &Self) -> bool {
            Arc::ptr_eq(&a.0, &b.0)
        }

        /// A stable identity pointer for this cell, for cycle detection (#381) — two `VmRef`s share it
        /// iff `ptr_eq`. Not for dereference; only for identity comparison / hashing.
        #[inline]
        pub fn as_ptr(&self) -> *const () {
            Arc::as_ptr(&self.0) as *const ()
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

#[cfg(not(feature = "portable"))]
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

// No unwinding on GBA (`panic = abort`), so `catch_unwind` is unavailable.
// Debug is used for host snapshot tests, not the runtime hot path, so a direct
// borrow is fine (a live borrow during Debug would abort, which never happens in
// practice on the single-threaded target).
#[cfg(feature = "portable")]
impl<T: fmt::Debug> fmt::Debug for VmRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RefCell {{ value: {:?} }}", &*self.borrow())
    }
}

#[cfg(all(test, feature = "send-values"))]
mod send_values_tests {
    use super::VmRef;

    /// Native codegen keeps `(*x.borrow())` temporaries alive to the end of a statement, so one
    /// statement can read the same cell twice: `{ a: x, b: n - x }`. That must not deadlock.
    #[test]
    fn two_reads_of_one_cell_at_once() {
        let x = VmRef::new(2.0_f64);
        let sum = *x.borrow() + *x.borrow();
        assert_eq!(sum, 4.0);
        *x.borrow_mut() = sum;
        assert_eq!(*x.borrow(), 4.0);
    }

    /// Readers on other threads still see writes, and a held read guard doesn't block another
    /// reader while a writer waits elsewhere.
    #[test]
    fn readers_and_writers_across_threads() {
        let x = VmRef::new(0_u32);
        let held = x.borrow();
        let writer = {
            let x = x.clone();
            std::thread::spawn(move || {
                *x.borrow_mut() += 1;
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(*x.borrow(), 0, "a second read while a writer waits");
        drop(held);
        writer.join().unwrap();
        assert_eq!(*x.borrow(), 1);
    }
}
