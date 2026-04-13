//! UI runtime: `h`, `Fragment`, vnode shapes compatible with Lattish, minimal hooks, and [`Host`].

mod hooks;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub use hooks::{
    alloc_root_id, current_root_id, install_host_for_root, native_create_root, native_use_memo,
    native_use_state, schedule_flush, unregister_root, with_host_for_root, HookState, LEGACY_ROOT_ID,
    RootId,
};

use tishlang_core::{ObjectMap, Value};

/// Sentinel string for `Fragment` (native). JS/Lattish uses `Symbol`; hosts compare via equality.
pub const FRAGMENT_SENTINEL: &str = "__tish_ui_Fragment__";

/// `Fragment` marker value for `h(Fragment, null, children)`.
pub fn fragment_value() -> Value {
    Value::String(FRAGMENT_SENTINEL.into())
}

/// Returns true if `tag` refers to [`fragment_value`].
pub fn is_fragment_tag(tag: &Value) -> bool {
    matches!(tag, Value::String(s) if s.as_ref() == FRAGMENT_SENTINEL)
}

/// `text(s)` helper — returns string as `Value::String` for JSX text nodes.
pub fn ui_text(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    Value::String(s.into())
}

/// Vnode factory: `h(tag, props, children)` (Lattish-compatible shape).
pub fn ui_h(args: &[Value]) -> Value {
    let tag = args.get(0).cloned().unwrap_or(Value::Null);
    let props = args.get(1).cloned().unwrap_or(Value::Null);
    let children_arg = args.get(2).cloned().unwrap_or(Value::Null);

    let children_vec = normalize_children_list(children_arg);

    if let Value::Function(f) = &tag {
        let mut merged = if matches!(props, Value::Null) {
            ObjectMap::default()
        } else if let Value::Object(obj) = props {
            obj.borrow().clone()
        } else {
            ObjectMap::default()
        };
        if !children_vec.is_empty() {
            merged.insert(
                Arc::from("children"),
                Value::Array(Rc::new(RefCell::new(children_vec.clone()))),
            );
        }
        return f(&[Value::Object(Rc::new(RefCell::new(merged)))]);
    }

    if is_fragment_tag(&tag) {
        return vnode_fragment(children_vec);
    }

    let tag_str: Arc<str> = match tag {
        Value::String(s) => s,
        _ => return Value::Null,
    };

    vnode_element(tag_str, props, children_vec)
}

fn normalize_children_list(children_arg: Value) -> Vec<Value> {
    match children_arg {
        Value::Null => vec![],
        Value::Array(a) => a.borrow().clone(),
        other => vec![other],
    }
}

fn vnode_element(tag: Arc<str>, props: Value, children: Vec<Value>) -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("tag"), Value::String(tag));
    m.insert(
        Arc::from("props"),
        if matches!(props, Value::Null) {
            Value::Null
        } else {
            props
        },
    );
    m.insert(
        Arc::from("children"),
        Value::Array(Rc::new(RefCell::new(children))),
    );
    m.insert(Arc::from("_el"), Value::Null);
    Value::Object(Rc::new(RefCell::new(m)))
}

fn vnode_fragment(children: Vec<Value>) -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("tag"), fragment_value());
    m.insert(Arc::from("props"), Value::Null);
    m.insert(
        Arc::from("children"),
        Value::Array(Rc::new(RefCell::new(children))),
    );
    m.insert(Arc::from("_el"), Value::Null);
    Value::Object(Rc::new(RefCell::new(m)))
}

/// Pluggable UI backend (Floem, DOM, SwiftUI, …). Main-thread / single-threaded by default.
pub trait Host {
    /// Apply a new root vnode (after each render flush).
    fn commit_root(&mut self, vnode: &Value);
    /// Content area width changed (e.g. window resize); default no-op.
    fn content_width_changed(&mut self, _width: f64) {}
    /// Called once from the main queue shortly after the window is ordered on-screen. Split /
    /// sidebar hosts can use this to re-layout when pane bounds were still provisional during the
    /// first commit.
    fn after_window_shown(&mut self) {}
}

/// No-op / test host that only stores the last committed tree.
pub struct HeadlessHost {
    pub last: Option<Value>,
}

impl Default for HeadlessHost {
    fn default() -> Self {
        Self { last: None }
    }
}

impl Host for HeadlessHost {
    fn commit_root(&mut self, vnode: &Value) {
        self.last = Some(vnode.clone());
    }
}

thread_local! {
    static ACTIVE_HOST: RefCell<Option<Box<dyn Host>>> = RefCell::new(None);
}

/// Install the thread-local host used by [`schedule_flush`] / `createRoot`.
pub fn install_thread_local_host(host: Box<dyn Host>) {
    ACTIVE_HOST.with(|c| {
        *c.borrow_mut() = Some(host);
    });
}

pub fn with_thread_local_host<R>(f: impl FnOnce(&mut dyn Host) -> R) -> Option<R> {
    ACTIVE_HOST.with(|c| {
        let mut opt = c.borrow_mut();
        match opt.as_deref_mut() {
            Some(host) => Some(f(host)),
            None => None,
        }
    })
}

/// Tag registry hook for future host-specific intrinsic mapping (HTML tag → component kind).
#[derive(Default)]
pub struct TagRegistry;

impl TagRegistry {
    pub fn new() -> Self {
        Self
    }
}

/// Placeholder for subset CSS / style object interpretation.
#[derive(Default)]
pub struct StyleInterpreter;

impl StyleInterpreter {
    pub fn new() -> Self {
        Self
    }
}
