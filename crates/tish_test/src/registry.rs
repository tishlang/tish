//! Suite tree registry — collect-then-run (Jest/Bun model).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tishlang_core::Value;

static ONLY_MODE: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestMode {
    Normal,
    Skip,
    Todo,
    Only,
    Failing,
}

#[derive(Clone)]
pub struct TestCase {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub callback: Value,
    pub mode: TestMode,
    pub timeout_ms: Option<u64>,
    pub concurrent: bool,
    pub serial: bool,
    pub tags: Vec<String>,
    pub file: String,
}

#[derive(Clone)]
pub struct SuiteNode {
    pub name: String,
    pub mode: TestMode,
    pub before_all: Vec<Value>,
    pub after_all: Vec<Value>,
    pub before_each: Vec<Value>,
    pub after_each: Vec<Value>,
    pub tests: Vec<TestCase>,
    pub children: Vec<SuiteNode>,
}

impl SuiteNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mode: TestMode::Normal,
            before_all: Vec::new(),
            after_all: Vec::new(),
            before_each: Vec::new(),
            after_each: Vec::new(),
            tests: Vec::new(),
            children: Vec::new(),
        }
    }
}

struct CollectState {
    root: SuiteNode,
    /// Stack of indices into nested children from root.
    path: Vec<usize>,
    current_file: String,
    collecting: bool,
}

impl CollectState {
    fn new() -> Self {
        Self {
            root: SuiteNode::new(""),
            path: Vec::new(),
            current_file: String::new(),
            collecting: false,
        }
    }

    fn current_mut(&mut self) -> &mut SuiteNode {
        let mut node = &mut self.root;
        for &i in &self.path {
            node = &mut node.children[i];
        }
        node
    }
}

thread_local! {
    static STATE: RefCell<CollectState> = RefCell::new(CollectState::new());
}

pub fn begin_collect(file: &str) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        *st = CollectState::new();
        st.current_file = file.to_string();
        st.collecting = true;
    });
    ONLY_MODE.store(false, Ordering::SeqCst);
}

pub fn end_collect() -> SuiteNode {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.collecting = false;
        std::mem::replace(&mut st.root, SuiteNode::new(""))
    })
}

pub fn is_collecting() -> bool {
    STATE.with(|s| s.borrow().collecting)
}

fn full_name_for(suite_path: &[String], test_name: &str) -> String {
    let mut parts = suite_path.to_vec();
    parts.push(test_name.to_string());
    parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" > ")
}

fn suite_path_names() -> Vec<String> {
    STATE.with(|s| {
        let st = s.borrow();
        let mut names = Vec::new();
        let mut node = &st.root;
        names.push(node.name.clone());
        for &i in &st.path {
            node = &node.children[i];
            names.push(node.name.clone());
        }
        names
    })
}

pub fn push_describe(name: &str, mode: TestMode, body: &Value) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if !st.collecting {
            return;
        }
        let child = SuiteNode {
            name: name.to_string(),
            mode,
            ..SuiteNode::new(name)
        };
        let idx = {
            let cur = st.current_mut();
            cur.children.push(child);
            cur.children.len() - 1
        };
        st.path.push(idx);
    });
    // Run describe body immediately (Jest model).
    let _ = tishlang_core::value_call(body, &[]);
    let _ = tishlang_core::take_pending_throw();
    STATE.with(|s| {
        s.borrow_mut().path.pop();
    });
}

pub fn add_test(
    name: &str,
    mode: TestMode,
    body: Value,
    timeout_ms: Option<u64>,
    tags: Vec<String>,
    concurrent: bool,
) {
    if mode == TestMode::Only {
        ONLY_MODE.store(true, Ordering::SeqCst);
    }
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if !st.collecting {
            return;
        }
        let path = {
            let mut names = Vec::new();
            let mut node = &st.root;
            names.push(node.name.clone());
            for &i in &st.path {
                node = &node.children[i];
                names.push(node.name.clone());
            }
            names
        };
        let file = st.current_file.clone();
        let case = TestCase {
            id: next_id(),
            name: name.to_string(),
            full_name: full_name_for(&path, name),
            callback: body,
            mode,
            timeout_ms,
            concurrent,
            serial: false,
            tags,
            file,
        };
        st.current_mut().tests.push(case);
    });
}

pub fn add_hook(kind: &str, body: Value) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if !st.collecting {
            return;
        }
        let cur = st.current_mut();
        match kind {
            "beforeAll" | "before" => cur.before_all.push(body),
            "afterAll" | "after" => cur.after_all.push(body),
            "beforeEach" => cur.before_each.push(body),
            "afterEach" => cur.after_each.push(body),
            _ => {}
        }
    });
}

pub fn only_mode() -> bool {
    ONLY_MODE.load(Ordering::SeqCst)
}

pub fn set_only_mode(v: bool) {
    ONLY_MODE.store(v, Ordering::SeqCst);
}

#[allow(dead_code)]
pub fn current_suite_path() -> Vec<String> {
    suite_path_names()
}
