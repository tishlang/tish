//! JSX lowering (compiler) and UI runtime (vnode + hooks + host) for cross-target Tish UI.
//!
//! - Feature **`compiler`**: AST → JS / Rust `h(...)` emission helpers (depends on `tishlang_ast`).
//! - Feature **`runtime`**: `Value`-based `h`, `Fragment`, hooks, and [`Host`] (depends on `tishlang_core`).

#[cfg(feature = "compiler")]
pub mod jsx;

#[cfg(feature = "runtime")]
pub mod runtime;

#[cfg(feature = "runtime")]
pub use runtime::{
    fragment_value, install_thread_local_host, native_create_root, native_use_state, ui_h,
    ui_text, with_thread_local_host, Host, HeadlessHost, FRAGMENT_SENTINEL,
};
