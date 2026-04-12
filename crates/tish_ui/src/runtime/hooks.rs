//! Minimal hook state: `useState`, `useMemo`, and render flush (Lattish-style cursor reset).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tishlang_core::{ObjectMap, Value};

use super::ACTIVE_HOST;

thread_local! {
    pub static HOOK: RefCell<HookState> = RefCell::new(HookState::default());
    static IN_FLUSH: Cell<bool> = Cell::new(false);
}

/// Hook storage for one `createRoot().render(App)` tree.
pub struct HookState {
    pub state_slots: Rc<RefCell<Vec<Value>>>,
    pub cursor: usize,
    pub root_app: Option<Value>,
    pub root_vnode: Option<Value>,
    pub flush_scheduled: bool,
    /// Per-slot: last dependency tuple snapshot and cached value from `useMemo`.
    pub memo_cache: Rc<RefCell<Vec<Option<(Vec<Value>, Value)>>>>,
    pub memo_cursor: usize,
}

impl Default for HookState {
    fn default() -> Self {
        Self {
            state_slots: Rc::new(RefCell::new(Vec::new())),
            cursor: 0,
            root_app: None,
            root_vnode: None,
            flush_scheduled: false,
            memo_cache: Rc::new(RefCell::new(Vec::new())),
            memo_cursor: 0,
        }
    }
}

impl HookState {
    pub fn reset_for_new_root(&mut self) {
        self.state_slots.borrow_mut().clear();
        self.cursor = 0;
        self.root_vnode = None;
        self.flush_scheduled = false;
        self.memo_cache.borrow_mut().clear();
        self.memo_cursor = 0;
    }
}

fn memo_dep_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if x.is_nan() && y.is_nan() {
                return true;
            }
            x == y
        }
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::Array(ax), Value::Array(bx)) => {
            let ab = ax.borrow();
            let bb = bx.borrow();
            if ab.len() != bb.len() {
                return false;
            }
            ab.iter()
                .zip(bb.iter())
                .all(|(x, y)| memo_dep_eq(x, y))
        }
        _ => false,
    }
}

fn memo_deps_unchanged(prev: &[Value], next: &[Value]) -> bool {
    prev.len() == next.len()
        && prev
            .iter()
            .zip(next.iter())
            .all(|(a, b)| memo_dep_eq(a, b))
}

/// `useState(initial)` → `[state, setState]` as a Tish array.
pub fn native_use_state(args: &[Value]) -> Value {
    let initial = args.first().cloned().unwrap_or(Value::Null);
    HOOK.with(|h| {
        let mut st = h.borrow_mut();
        let i = st.cursor;
        st.cursor += 1;
        let slots = Rc::clone(&st.state_slots);
        while i >= slots.borrow().len() {
            slots.borrow_mut().push(initial.clone());
        }
        let current = slots.borrow()[i].clone();
        let idx = i;
        let setter = Value::Function(Rc::new(move |a: &[Value]| {
            let new_v = a.first().cloned().unwrap_or(Value::Null);
            slots.borrow_mut()[idx] = new_v;
            schedule_flush();
            Value::Null
        }));
        Value::Array(Rc::new(RefCell::new(vec![current, setter])))
    })
}

/// `useMemo(factory, deps?)` — caches `factory()` until `deps` changes (shallow compare per slot).
///
/// Dependency comparison supports `number`, `string`, `bool`, `null`, and nested arrays of those
/// (e.g. `[a, b]`). **Function** identity in deps is not supported.
pub fn native_use_memo(args: &[Value]) -> Value {
    let Some(Value::Function(factory)) = args.first() else {
        return Value::Null;
    };
    let factory = Rc::clone(factory);
    let deps: Vec<Value> = match args.get(1) {
        Some(Value::Array(a)) => a.borrow().clone(),
        Some(other) => vec![other.clone()],
        None => vec![],
    };

    HOOK.with(|h| {
        let mut st = h.borrow_mut();
        let i = st.memo_cursor;
        st.memo_cursor += 1;
        let cache = Rc::clone(&st.memo_cache);
        let mut c = cache.borrow_mut();
        while c.len() <= i {
            c.push(None);
        }
        let reuse = match &c[i] {
            Some((old_deps, _)) => memo_deps_unchanged(old_deps, &deps),
            None => false,
        };
        if reuse {
            return c[i].as_ref().unwrap().1.clone();
        }
        let produced = factory(&[]);
        c[i] = Some((deps, produced.clone()));
        produced
    })
}

/// `createRoot(container)` → `{ render: (App) => { ... } }` (container ignored for headless native).
pub fn native_create_root(args: &[Value]) -> Value {
    let _container = args.first();
    let render_fn = Value::Function(Rc::new(|app_args: &[Value]| {
        let app = app_args.first().cloned().unwrap_or(Value::Null);
        HOOK.with(|h| {
            let mut st = h.borrow_mut();
            st.reset_for_new_root();
            st.root_app = Some(app);
            st.flush_scheduled = true;
        });
        drain_flush_queue();
        Value::Null
    }));
    Value::Object(Rc::new(RefCell::new(ObjectMap::from([(
        std::sync::Arc::from("render"),
        render_fn,
    )]))))
}

/// Request a re-render (coalesced; safe if called during flush).
pub fn schedule_flush() {
    HOOK.with(|h| {
        h.borrow_mut().flush_scheduled = true;
    });
    if IN_FLUSH.get() {
        return;
    }
    drain_flush_queue();
}

fn drain_flush_queue() {
    loop {
        let run = HOOK.with(|h| {
            let mut st = h.borrow_mut();
            if st.flush_scheduled {
                st.flush_scheduled = false;
                true
            } else {
                false
            }
        });
        if !run {
            break;
        }
        IN_FLUSH.set(true);
        // Clone the app `NativeFn` and reset the hook cursor without holding `HOOK` across `f(&[])`.
        // Component code (e.g. `useState`) borrows `HOOK` again; a nested `borrow_mut` would panic.
        let app_fn = HOOK.with(|h| {
            let mut st = h.borrow_mut();
            st.cursor = 0;
            st.memo_cursor = 0;
            let Some(app) = st.root_app.clone() else {
                return None;
            };
            let Value::Function(f) = app else {
                return None;
            };
            Some(f)
        });
        if let Some(f) = app_fn {
            let tree = f(&[]);
            HOOK.with(|h| {
                let mut st = h.borrow_mut();
                st.root_vnode = Some(tree.clone());
                ACTIVE_HOST.with(|host_cell| {
                    let mut host_opt = host_cell.borrow_mut();
                    if let Some(host) = host_opt.as_deref_mut() {
                        host.commit_root(&tree);
                    }
                });
            });
        }
        IN_FLUSH.set(false);
    }
}
