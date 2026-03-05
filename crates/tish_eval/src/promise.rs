//! ECMA-262 §27.2 Promise implementation for Tish interpreter.
//! Requires tokio (http feature) for block_on and microtask scheduling.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tokio::sync::oneshot;

use crate::value::Value;

/// A reaction from .then or .finally.
pub enum Reaction {
    Then(
        Option<Value>,   // onFulfilled
        Option<Value>,   // onRejected
        PromiseResolver, // resolve
        PromiseResolver, // reject
    ),
    Finally(Value, PromiseResolver, PromiseResolver), // onFinally, resolve, reject
}

/// Promise state: Pending, Fulfilled, or Rejected.
pub enum PromiseState {
    Pending {
        tx: Option<oneshot::Sender<Result<Value, Value>>>,
        reactions: VecDeque<Reaction>,
    },
    Fulfilled(Value),
    Rejected(Value),
}

/// Internal state for a Promise, shared via Rc<RefCell<>>.
pub type PromiseStateRef = Rc<RefCell<PromiseState>>;

/// A promise value holds state and the receiver for blocking until settled.
#[derive(Clone)]
pub struct PromiseRef {
    pub state: PromiseStateRef,
    pub rx: Rc<RefCell<Option<oneshot::Receiver<Result<Value, Value>>>>>,
}

/// Data for resolve/reject callables passed to executor.
#[derive(Clone)]
pub struct PromiseResolver {
    pub state: PromiseStateRef,
    pub is_resolve: bool,
}

/// Extract PromiseResolver from Value::PromiseResolver. Panics if not.
pub fn extract_resolvers(resolve: &Value, reject: &Value) -> (PromiseResolver, PromiseResolver) {
    let r = match resolve {
        Value::PromiseResolver(x) => x.clone(),
        _ => panic!("expected PromiseResolver"),
    };
    let j = match reject {
        Value::PromiseResolver(x) => x.clone(),
        _ => panic!("expected PromiseResolver"),
    };
    (r, j)
}

/// Create a new pending Promise. Returns (promise_value, resolve_value, reject_value).
/// The executor will be called with (resolve, reject). Resolve/reject can only be called once.
pub fn create_promise() -> (Value, Value, Value) {
    let (tx, rx) = oneshot::channel();
    let state = Rc::new(RefCell::new(PromiseState::Pending {
        tx: Some(tx),
        reactions: VecDeque::new(),
    }));
    let rx_cell = Rc::new(RefCell::new(Some(rx)));
    let resolve = Value::PromiseResolver(PromiseResolver {
        state: Rc::clone(&state),
        is_resolve: true,
    });
    let reject = Value::PromiseResolver(PromiseResolver {
        state: Rc::clone(&state),
        is_resolve: false,
    });
    let promise = Value::Promise(PromiseRef {
        state: Rc::clone(&state),
        rx: rx_cell,
    });
    (promise, resolve, reject)
}

/// Result of settling: reactions to run (caller must run them with evaluator).
pub type SettleResult = (Value, bool, Vec<Reaction>);

/// Settle the promise (resolve or reject). Called when PromiseResolver is invoked.
/// Returns Ok((value, is_fulfilled, reactions)) if settled; reactions should be run by caller.
/// Returns Err(msg) if already settled.
pub fn settle_promise(
    resolver: &PromiseResolver,
    value: Value,
    is_resolve: bool,
) -> Result<SettleResult, String> {
    let (tx, reactions) = {
        let mut state = resolver.state.borrow_mut();
        match std::mem::replace(&mut *state, PromiseState::Fulfilled(Value::Null)) {
            PromiseState::Pending { tx, reactions } => {
                *state = if is_resolve {
                    PromiseState::Fulfilled(value.clone())
                } else {
                    PromiseState::Rejected(value.clone())
                };
                (tx, reactions.into_iter().collect())
            }
            s @ PromiseState::Fulfilled(_) | s @ PromiseState::Rejected(_) => {
                *state = s;
                return Err("Promise already settled".to_string());
            }
        }
    };
    if let Some(tx) = tx {
        let result = if is_resolve { Ok(value.clone()) } else { Err(value.clone()) };
        let _ = tx.send(result);
    }
    Ok((value, is_resolve, reactions))
}

/// Add a reaction to a pending promise. Caller must ensure promise is Pending.
pub fn add_reaction(state: &PromiseStateRef, reaction: Reaction) {
    let mut s = state.borrow_mut();
    if let PromiseState::Pending { reactions, .. } = &mut *s {
        reactions.push_back(reaction);
    }
}

// Clone for Reaction
impl Clone for Reaction {
    fn clone(&self) -> Self {
        match self {
            Reaction::Then(a, b, r1, r2) => Reaction::Then(a.clone(), b.clone(), r1.clone(), r2.clone()),
            Reaction::Finally(f, r1, r2) => Reaction::Finally(f.clone(), r1.clone(), r2.clone()),
        }
    }
}

/// Result of awaiting a promise: fulfilled value, or rejection/error.
#[derive(Debug)]
pub enum PromiseAwaitResult {
    Fulfilled(Value),
    Rejected(Value),
    Error(String),
}

/// Block until the promise settles.
pub fn block_until_settled(promise_ref: &PromiseRef) -> PromiseAwaitResult {
    let maybe_rx = promise_ref.rx.borrow_mut().take();
    if let Some(rx) = maybe_rx {
        let result = crate::http::RUNTIME.with(|rt| rt.block_on(rx));
        match result {
            Ok(Ok(v)) => PromiseAwaitResult::Fulfilled(v),
            Ok(Err(v)) => PromiseAwaitResult::Rejected(v),
            Err(_) => PromiseAwaitResult::Error(
                "Promise channel dropped before settlement".to_string(),
            ),
        }
    } else {
        let state = promise_ref.state.borrow();
        match &*state {
            PromiseState::Fulfilled(v) => PromiseAwaitResult::Fulfilled(v.clone()),
            PromiseState::Rejected(v) => PromiseAwaitResult::Rejected(v.clone()),
            PromiseState::Pending { .. } => PromiseAwaitResult::Error(
                "Promise receiver already consumed".to_string(),
            ),
        }
    }
}
