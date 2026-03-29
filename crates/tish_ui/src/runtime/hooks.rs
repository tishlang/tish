//! Minimal hook state: `useState` + render flush (Lattish-style cursor reset).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tishlang_core::{ObjectMap, Value};

use super::ACTIVE_HOST;

thread_local! {
    pub static HOOK: RefCell<HookState> = RefCell::new(HookState::default());
    static IN_FLUSH: Cell<bool> = Cell::new(false);
}

#[derive(Default)]
pub struct HookState {
    pub state_slots: Rc<RefCell<Vec<Value>>>,
    pub cursor: usize,
    pub root_app: Option<Value>,
    pub root_vnode: Option<Value>,
    pub flush_scheduled: bool,
}

impl HookState {
    pub fn reset_for_new_root(&mut self) {
        self.state_slots.borrow_mut().clear();
        self.cursor = 0;
        self.root_vnode = None;
        self.flush_scheduled = false;
    }
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
        HOOK.with(|h| {
            let mut st = h.borrow_mut();
            st.cursor = 0;
            let Some(app) = st.root_app.clone() else {
                return;
            };
            let Value::Function(f) = app else {
                return;
            };
            let tree = f(&[]);
            st.root_vnode = Some(tree.clone());
            ACTIVE_HOST.with(|host_cell| {
                let mut host_opt = host_cell.borrow_mut();
                if let Some(host) = host_opt.as_deref_mut() {
                    host.commit_root(&tree);
                }
            });
        });
        IN_FLUSH.set(false);
    }
}
