//! Minimal hook state: `useState`, `useMemo`, and render flush (Lattish-style cursor reset).
//! Supports multiple independent roots (`RootId`) in one thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use tishlang_core::{ObjectMap, Value};

use super::Host;

/// Opaque id for one `createRoot().render(App)` tree in this thread.
pub type RootId = u64;

/// First root: `install_thread_local_host` and `native_create_root` without an id argument.
pub const LEGACY_ROOT_ID: RootId = 1;

thread_local! {
    static HOOKS: RefCell<HashMap<RootId, HookState>> = RefCell::new(HashMap::new());
    static CURRENT_ROOT: Cell<Option<RootId>> = Cell::new(None);
    static HOSTS: RefCell<HashMap<RootId, Box<dyn Host>>> = RefCell::new(HashMap::new());
    static NEXT_DYNAMIC_ROOT_ID: Cell<RootId> = Cell::new(2);
    static IN_FLUSH: Cell<bool> = Cell::new(false);
}

/// Allocate an id for an additional in-process window (starts at 2; 1 is legacy primary).
pub fn alloc_root_id() -> RootId {
    NEXT_DYNAMIC_ROOT_ID.with(|n| {
        let id = n.get();
        n.set(id.saturating_add(1).max(2));
        id
    })
}

fn ensure_hook_entry(root_id: RootId) {
    HOOKS.with(|h| {
        h.borrow_mut().entry(root_id).or_default();
    });
}

/// Install the host for a specific root. Replaces any previous host for that id.
pub fn install_host_for_root(root_id: RootId, host: Box<dyn Host>) {
    ensure_hook_entry(root_id);
    HOSTS.with(|h| {
        h.borrow_mut().insert(root_id, host);
    });
}

/// Legacy: install host for [`LEGACY_ROOT_ID`] (same as `macos.run` / single-window tools).
#[allow(dead_code)] // Emitted Rust / hosts call via `tishlang_ui` re-exports; unused inside this crate.
pub fn install_thread_local_host(host: Box<dyn Host>) {
    install_host_for_root(LEGACY_ROOT_ID, host);
}

pub fn unregister_root(root_id: RootId) {
    HOOKS.with(|h| {
        if let Some(st) = h.borrow_mut().remove(&root_id) {
            run_all_effect_cleanups(st.effect_cells.as_ref());
        }
    });
    HOSTS.with(|h| {
        h.borrow_mut().remove(&root_id);
    });
}

pub fn with_host_for_root<R>(root_id: RootId, f: impl FnOnce(&mut dyn Host) -> R) -> Option<R> {
    HOSTS.with(|c| {
        let mut m = c.borrow_mut();
        m.get_mut(&root_id).map(|host| f(host.as_mut()))
    })
}

/// Prefer [`with_host_for_root`]; kept for call sites that assume a single primary root.
#[allow(dead_code)]
pub fn with_thread_local_host<R>(f: impl FnOnce(&mut dyn Host) -> R) -> Option<R> {
    with_host_for_root(LEGACY_ROOT_ID, f)
}

/// Root currently rendering or running hook flush (`None` outside that scope).
pub fn current_root_id() -> Option<RootId> {
    CURRENT_ROOT.get()
}

/// One `useEffect` slot: committed dependency snapshot and optional cleanup from the last run.
#[derive(Default)]
struct EffectCell {
    committed_deps: Option<Vec<Value>>,
    cleanup: Option<Value>,
}

struct PendingEffect {
    slot: usize,
    effect_fn: Value,
    new_deps: Vec<Value>,
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
    effect_cells: Rc<RefCell<Vec<EffectCell>>>,
    effect_cursor: usize,
    pending_effects: Rc<RefCell<Vec<PendingEffect>>>,
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
            effect_cells: Rc::new(RefCell::new(Vec::new())),
            effect_cursor: 0,
            pending_effects: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

fn run_all_effect_cleanups(cells: &RefCell<Vec<EffectCell>>) {
    for cell in cells.borrow_mut().iter_mut() {
        if let Some(c) = cell.cleanup.take() {
            if let Value::Function(f) = c {
                let _ = f(&[]);
            }
        }
    }
}

impl HookState {
    pub fn reset_for_new_root(&mut self) {
        run_all_effect_cleanups(self.effect_cells.as_ref());
        self.effect_cells.borrow_mut().clear();
        self.effect_cursor = 0;
        self.pending_effects.borrow_mut().clear();
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

fn root_id_for_hooks() -> RootId {
    CURRENT_ROOT.get().unwrap_or(LEGACY_ROOT_ID)
}

/// `useState(initial)` → `[state, setState]` as a Tish array.
pub fn native_use_state(args: &[Value]) -> Value {
    let initial = args.first().cloned().unwrap_or(Value::Null);
    let root_id = root_id_for_hooks();
    HOOKS.with(|h| {
        let mut map = h.borrow_mut();
        let st = map.entry(root_id).or_default();
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
            HOOKS.with(|hooks| {
                if let Some(st) = hooks.borrow_mut().get_mut(&root_id) {
                    st.state_slots.borrow_mut()[idx] = new_v;
                    st.flush_scheduled = true;
                }
            });
            if !IN_FLUSH.get() {
                drain_flush_queue();
            }
            Value::Null
        }));
        Value::Array(Rc::new(RefCell::new(vec![current, setter])))
    })
}

/// `useMemo(factory, deps?)` — caches `factory()` until `deps` changes (shallow compare per slot).
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

    let root_id = root_id_for_hooks();
    HOOKS.with(|h| {
        let mut map = h.borrow_mut();
        let st = map.entry(root_id).or_default();
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

/// `useEffect(effect, deps?)` — runs `effect` after the host commits the tree; compares `deps` like `useMemo`.
/// If `effect` returns a function, it is called before the next run or on root teardown (`render` replacement / [`unregister_root`]).
pub fn native_use_effect(args: &[Value]) -> Value {
    let Some(Value::Function(effect_fn)) = args.first() else {
        return Value::Null;
    };
    let effect_fn = Rc::clone(effect_fn);
    let deps: Vec<Value> = match args.get(1) {
        Some(Value::Array(a)) => a.borrow().clone(),
        Some(other) => vec![other.clone()],
        None => vec![],
    };

    let root_id = root_id_for_hooks();
    HOOKS.with(|h| {
        let mut map = h.borrow_mut();
        let st = map.entry(root_id).or_default();
        let i = st.effect_cursor;
        st.effect_cursor += 1;
        let cells = Rc::clone(&st.effect_cells);
        let mut cells_b = cells.borrow_mut();
        while cells_b.len() <= i {
            cells_b.push(EffectCell::default());
        }
        let should_run = match &cells_b[i].committed_deps {
            None => true,
            Some(old) => !memo_deps_unchanged(old, &deps),
        };
        drop(cells_b);

        if should_run {
            st.pending_effects.borrow_mut().push(PendingEffect {
                slot: i,
                effect_fn: Value::Function(effect_fn),
                new_deps: deps,
            });
        }
        Value::Null
    })
}

fn flush_pending_effects(root_id: RootId) {
    let pending: Vec<PendingEffect> = HOOKS.with(|h| {
        h.borrow_mut()
            .get_mut(&root_id)
            .map(|st| std::mem::take(&mut *st.pending_effects.borrow_mut()))
            .unwrap_or_default()
    });
    let cells_rc = HOOKS.with(|h| {
        h.borrow()
            .get(&root_id)
            .map(|st| Rc::clone(&st.effect_cells))
    });
    let Some(cells_rc) = cells_rc else {
        return;
    };

    for p in pending {
        let mut cells = cells_rc.borrow_mut();
        while cells.len() <= p.slot {
            cells.push(EffectCell::default());
        }
        let cell = &mut cells[p.slot];
        if let Some(c) = cell.cleanup.take() {
            if let Value::Function(f) = c {
                let _ = f(&[]);
            }
        }
        let run_result = if let Value::Function(f) = &p.effect_fn {
            f(&[])
        } else {
            Value::Null
        };
        cell.cleanup = match run_result {
            Value::Function(f) => Some(Value::Function(f)),
            _ => None,
        };
        cell.committed_deps = Some(p.new_deps);
    }
}

fn parse_root_id_arg(args: &[Value]) -> RootId {
    match args.first() {
        Some(Value::Number(n)) if n.is_finite() && *n >= 1.0 && n.fract() == 0.0 => *n as u64,
        _ => LEGACY_ROOT_ID,
    }
}

/// `createRoot(container?)` or `createRoot(rootId)` → `{ render: (App) => { ... } }`.
/// Pass a positive integer as the first argument to bind this root to a host installed via
/// [`install_host_for_root`].
pub fn native_create_root(args: &[Value]) -> Value {
    let root_id = parse_root_id_arg(args);
    ensure_hook_entry(root_id);
    let render_fn = Value::Function(Rc::new(move |app_args: &[Value]| {
        let app = app_args.first().cloned().unwrap_or(Value::Null);
        HOOKS.with(|h| {
            let mut map = h.borrow_mut();
            let st = map.entry(root_id).or_default();
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
    let root_id = root_id_for_hooks();
    HOOKS.with(|h| {
        if let Some(st) = h.borrow_mut().get_mut(&root_id) {
            st.flush_scheduled = true;
        }
    });
    if IN_FLUSH.get() {
        return;
    }
    drain_flush_queue();
}

fn drain_flush_queue() {
    loop {
        let root_id = HOOKS.with(|h| {
            h.borrow()
                .iter()
                .find(|(_, st)| st.flush_scheduled)
                .map(|(id, _)| *id)
        });
        let Some(root_id) = root_id else {
            break;
        };

        IN_FLUSH.set(true);
        CURRENT_ROOT.set(Some(root_id));
        HOOKS.with(|h| {
            if let Some(st) = h.borrow_mut().get_mut(&root_id) {
                st.flush_scheduled = false;
            }
        });

        let app_fn = HOOKS.with(|h| {
            let mut map = h.borrow_mut();
            let st = map.get_mut(&root_id)?;
            st.cursor = 0;
            st.memo_cursor = 0;
            st.effect_cursor = 0;
            st.pending_effects.borrow_mut().clear();
            let app = st.root_app.clone()?;
            let Value::Function(f) = app else {
                return None;
            };
            Some(f)
        });

        if let Some(f) = app_fn {
            let tree = f(&[]);
            HOOKS.with(|h| {
                let mut map = h.borrow_mut();
                if let Some(st) = map.get_mut(&root_id) {
                    st.root_vnode = Some(tree.clone());
                    HOSTS.with(|hosts| {
                        let mut hm = hosts.borrow_mut();
                        if let Some(host) = hm.get_mut(&root_id) {
                            host.commit_root(&tree);
                        }
                    });
                }
            });
            IN_FLUSH.set(false);
            flush_pending_effects(root_id);
        }

        CURRENT_ROOT.set(None);
        IN_FLUSH.set(false);
    }
}
