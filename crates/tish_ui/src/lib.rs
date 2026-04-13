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
    alloc_root_id, current_root_id, fragment_value, install_host_for_root, install_thread_local_host,
    native_create_root, native_use_effect, native_use_memo, native_use_state, ui_h, ui_text,
    unregister_root,
    with_host_for_root, with_thread_local_host, HeadlessHost, Host, FRAGMENT_SENTINEL, LEGACY_ROOT_ID,
    RootId,
};
