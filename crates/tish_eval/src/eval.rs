//! Tree-walk evaluator for Tish.

#![allow(clippy::type_complexity, clippy::cloned_ref_to_slice_refs)]

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use tishlang_ast::{
    BinOp, CompoundOp, ExportDeclaration, Expr, FunParam, ImportSpecifier, Literal,
    LogicalAssignOp, MemberProp, Span, Statement, UnaryOp,
};

#[cfg(any(feature = "fs", feature = "process"))]
use crate::natives;
use ahash::AHashMap;

use crate::value::{
    eval_object_get, eval_object_has, eval_object_set, EvalObjectData, PropMap, Value,
};

// #203: cursor cache for O(1)/near-O(1) character indexing in the interpreter. Twin of the one in
// `tishlang_builtins::string` (used by the native/VM backends); duplicated here because the
// interpreter has its own `Value`/`Arc<str>` string type rather than `tishlang_core`'s `ArcStr`. tish
// strings are UTF-8, so `chars().nth(i)` is O(i) — turning indexed/strided scans into O(n^2). For each
// recently-indexed string we cache whether it is all-ASCII (then a character index equals a byte index
// → O(1)) plus a forward cursor (so non-ASCII sequential/strided scans advance from the last position).
// Safety: the entry holds an `Arc<str>` CLONE, keeping the allocation alive so its data pointer can't
// be reused while cached (no ABA), and since strings are immutable the cached ASCII flag stays valid.
struct EvalCharCursor {
    s: Arc<str>,
    ascii: bool,
    len_chars: usize,
    char_idx: usize,
    byte_off: usize,
}

thread_local! {
    static EVAL_INDEX_CURSOR: RefCell<Option<EvalCharCursor>> = const { RefCell::new(None) };
}

fn with_eval_cursor<R>(s: &Arc<str>, f: impl FnOnce(&mut EvalCharCursor, &Arc<str>) -> R) -> R {
    EVAL_INDEX_CURSOR.with(|cell| {
        let mut slot = cell.borrow_mut();
        let hit = matches!(slot.as_ref(), Some(c)
            if std::ptr::eq(c.s.as_bytes().as_ptr(), s.as_bytes().as_ptr()) && c.s.len() == s.len());
        if !hit {
            let ascii = s.as_bytes().is_ascii();
            let len_chars = if ascii { s.len() } else { s.chars().count() };
            *slot = Some(EvalCharCursor {
                s: Arc::clone(s),
                ascii,
                len_chars,
                char_idx: 0,
                byte_off: 0,
            });
        }
        f(slot.as_mut().unwrap(), s)
    })
}

/// Character (Unicode scalar) at index `idx` via the cursor cache. Identical results to
/// `s.chars().nth(idx)`, but O(1) for ASCII and near-O(1) for forward/strided scans.
fn nth_char_cached(s: &Arc<str>, idx: usize) -> Option<char> {
    with_eval_cursor(s, |c, s| {
        if c.ascii {
            return s.as_bytes().get(idx).map(|&b| b as char);
        }
        let (base_idx, base_off) = if idx >= c.char_idx {
            (c.char_idx, c.byte_off)
        } else {
            (0, 0)
        };
        match s[base_off..].char_indices().nth(idx - base_idx) {
            Some((rel_off, ch)) => {
                c.char_idx = idx;
                c.byte_off = base_off + rel_off;
                Some(ch)
            }
            None => None,
        }
    })
}

/// Character (Unicode scalar) count via the cursor cache — O(1) after the first call on a string.
fn char_count_cached(s: &Arc<str>) -> usize {
    with_eval_cursor(s, |c, _| c.len_chars)
}

pub struct Scope {
    // Scope vars: order is never observed (no Object.keys over a scope), so use a fast
    // unordered aHash map — NOT the object-strings PropMap (an insertion-ordered IndexMap),
    // which would pay SipHash + ordered-bucket overhead on every variable lookup.
    vars: AHashMap<Arc<str>, Value>,
    consts: ahash::AHashSet<Arc<str>>,
    parent: Option<Rc<std::cell::RefCell<Scope>>>,
}

// #186 / string_build: amortized O(1) `acc += x`. tish strings are an immutable `Arc<str>`, so the
// generic `acc = acc + x` allocates a fresh String and copies the whole accumulator each time → O(n^2)
// over a build loop. Instead we keep the accumulator in a growable `String` ("the pending builder")
// and `push_str` onto it in O(1), writing it back to the scope slot ("flushing") the moment the
// variable is observed. JS strings are immutable VALUES, so soundness requires that any read sees the
// full current string: every read flushes first (see `flush_pending_for` at `Expr::Ident` and the
// other read sites). The builder is keyed by the EXACT owning scope (captured `Rc`) plus name, so a
// shadowing inner `acc` never appends onto an outer `acc`'s buffer. Shared across nested evaluators
// (function calls / closures) via `Rc` so a callee that reads the variable flushes the same buffer.
struct PendingAppend {
    /// The exact scope that owns the accumulator variable (not necessarily the current scope).
    scope: Rc<std::cell::RefCell<Scope>>,
    name: Arc<str>,
    /// The true current value of the accumulator while buffered; the scope slot is stale until flush.
    buf: String,
}

#[derive(Default)]
struct StringBuilderState {
    /// Fast-path guard: a single `Cell<bool>` load lets the hot identifier-read path skip the
    /// `RefCell` borrow entirely when no builder is active (the case for all non-building programs).
    active: Cell<bool>,
    pending: RefCell<Option<PendingAppend>>,
}

/// A reference-counted lexical scope. A `Value::Function` captures one of these at creation
/// (the *defining* scope) so calls resolve free variables lexically — real closures.
pub type ScopeRef = Rc<std::cell::RefCell<Scope>>;

impl Scope {
    fn new() -> Rc<std::cell::RefCell<Self>> {
        Rc::new(std::cell::RefCell::new(Self {
            vars: AHashMap::default(),
            consts: ahash::AHashSet::default(),
            parent: None,
        }))
    }

    fn child(parent: Rc<std::cell::RefCell<Scope>>) -> Rc<std::cell::RefCell<Self>> {
        Rc::new(std::cell::RefCell::new(Self {
            vars: AHashMap::default(),
            consts: ahash::AHashSet::default(),
            parent: Some(parent),
        }))
    }

    fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.get(name) {
            return Some(v.clone());
        }
        if let Some(ref parent) = self.parent {
            return parent.borrow().get(name);
        }
        None
    }

    fn set(&mut self, name: Arc<str>, value: Value, mutable: bool) {
        if !mutable {
            self.consts.insert(Arc::clone(&name));
        }
        self.vars.insert(name, value);
    }

    fn assign(&mut self, name: &str, value: Value) -> Result<bool, String> {
        if let Some(existing) = self.vars.get_mut(name) {
            if self.consts.contains(name) {
                return Err(format!("Cannot assign to const variable: {}", name));
            }
            *existing = value;
            return Ok(true);
        }
        if let Some(ref parent) = self.parent {
            return parent.borrow_mut().assign(name, value);
        }
        Ok(false)
    }
}

pub struct Evaluator {
    scope: Rc<std::cell::RefCell<Scope>>,
    /// Cache of evaluated modules: canonical path -> exports object
    module_cache: Rc<RefCell<HashMap<PathBuf, Value>>>,
    /// Directory of the file currently being evaluated (for resolving relative imports)
    current_dir: RefCell<Option<PathBuf>>,
    /// Extra `tish:*` builtins from `TishNativeModule::virtual_builtin_modules` (shared across nested evaluators).
    virtual_builtins: Rc<RefCell<HashMap<Arc<str>, Value>>>,
    /// String-builder state for amortized O(1) `acc += x` (see [`StringBuilderState`]). Shared across
    /// nested evaluators so a called function/closure that reads the accumulator flushes the buffer.
    string_builder: Rc<StringBuilderState>,
    /// Current user-function call depth, SHARED (`Rc`) across every nested call-frame evaluator so it
    /// tracks total recursion. Past [`Evaluator::max_call_depth`] a call throws a catchable
    /// `RangeError` instead of growing the stack toward OOM/abort (#381).
    call_depth: Rc<std::cell::Cell<usize>>,
    /// Recursion ceiling: past this many nested user-function frames a call throws a catchable
    /// `RangeError('Maximum call stack size exceeded')`, matching JS, rather than aborting the process.
    /// Defaults to [`DEFAULT_MAX_CALL_DEPTH`], overridable via `TISH_MAX_CALL_DEPTH`.
    max_call_depth: usize,
}

/// Default recursion ceiling: far deeper than any real non-pathological recursion, yet below where
/// `stacker`'s growth would exhaust memory (the accompanying `stacker::maybe_grow` was verified safe
/// to depth 20000). Override with `TISH_MAX_CALL_DEPTH`.
const DEFAULT_MAX_CALL_DEPTH: usize = 20_000;

fn env_max_call_depth() -> usize {
    std::env::var("TISH_MAX_CALL_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_CALL_DEPTH)
}

impl Evaluator {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        use crate::natives;

        let scope = Scope::new();
        {
            let mut s = scope.borrow_mut();
            let mut console = PropMap::with_capacity(5);
            console.insert("debug".into(), Value::Native(natives::console_debug));
            console.insert("info".into(), Value::Native(natives::console_info));
            console.insert("log".into(), Value::Native(natives::console_log));
            console.insert("warn".into(), Value::Native(natives::console_warn));
            console.insert("error".into(), Value::Native(natives::console_error));
            s.set(
                "console".into(),
                Value::object(console),
                true,
            );
            s.set("parseInt".into(), Value::Native(natives::parse_int), true);
            s.set(
                "parseFloat".into(),
                Value::Native(natives::parse_float),
                true,
            );
            s.set("decodeURI".into(), Value::Native(natives::decode_uri), true);
            s.set("encodeURI".into(), Value::Native(natives::encode_uri), true);
            s.set(
                "htmlEscape".into(),
                Value::Native(natives::html_escape),
                true,
            );
            s.set(
                "Boolean".into(),
                Value::Native(natives::boolean_native),
                true,
            );
            s.set("isFinite".into(), Value::Native(natives::is_finite), true);
            s.set("isNaN".into(), Value::Native(natives::is_nan), true);
            s.set("Infinity".into(), Value::Number(f64::INFINITY), true);
            s.set("NaN".into(), Value::Number(f64::NAN), true);
            let mut math = PropMap::with_capacity(18);
            math.insert("abs".into(), Value::Native(natives::math_abs));
            math.insert("sqrt".into(), Value::Native(natives::math_sqrt));
            math.insert("min".into(), Value::Native(natives::math_min));
            math.insert("max".into(), Value::Native(natives::math_max));
            math.insert("floor".into(), Value::Native(natives::math_floor));
            math.insert("ceil".into(), Value::Native(natives::math_ceil));
            math.insert("round".into(), Value::Native(natives::math_round));
            math.insert("random".into(), Value::Native(natives::math_random));
            math.insert("pow".into(), Value::Native(natives::math_pow));
            math.insert("hypot".into(), Value::Native(natives::math_hypot));
            math.insert("asin".into(), Value::Native(natives::math_asin));
            math.insert("acos".into(), Value::Native(natives::math_acos));
            math.insert("atan".into(), Value::Native(natives::math_atan));
            math.insert("atan2".into(), Value::Native(natives::math_atan2));
            math.insert("sin".into(), Value::Native(natives::math_sin));
            math.insert("cos".into(), Value::Native(natives::math_cos));
            math.insert("tan".into(), Value::Native(natives::math_tan));
            math.insert("log".into(), Value::Native(natives::math_log));
            math.insert("exp".into(), Value::Native(natives::math_exp));
            math.insert("sign".into(), Value::Native(natives::math_sign));
            math.insert("trunc".into(), Value::Native(natives::math_trunc));
            math.insert("sinh".into(), Value::Native(natives::math_sinh));
            math.insert("cosh".into(), Value::Native(natives::math_cosh));
            math.insert("tanh".into(), Value::Native(natives::math_tanh));
            math.insert("asinh".into(), Value::Native(natives::math_asinh));
            math.insert("acosh".into(), Value::Native(natives::math_acosh));
            math.insert("atanh".into(), Value::Native(natives::math_atanh));
            math.insert("cbrt".into(), Value::Native(natives::math_cbrt));
            math.insert("log2".into(), Value::Native(natives::math_log2));
            math.insert("log10".into(), Value::Native(natives::math_log10));
            math.insert("PI".into(), Value::Number(std::f64::consts::PI));
            math.insert("E".into(), Value::Number(std::f64::consts::E));
            s.set(
                "Math".into(),
                Value::object(math),
                true,
            );

            let mut json = PropMap::with_capacity(2);
            json.insert("parse".into(), Value::Native(Self::json_parse_native));
            json.insert(
                "stringify".into(),
                Value::Native(Self::json_stringify_native),
            );
            s.set(
                "JSON".into(),
                Value::object(json),
                true,
            );

            // Bare `process` global (node-compatible), mirroring the VM. `process.argv` reads the
            // configurable argv so `tish run <file> [args...]` reaches the script. #88
            #[cfg(feature = "process")]
            {
                let mut process_obj = PropMap::default();
                process_obj.insert("exit".into(), Value::Native(natives::process_exit));
                process_obj.insert("cwd".into(), Value::Native(natives::process_cwd));
                process_obj.insert("exec".into(), Value::Native(natives::process_exec));
                process_obj.insert("execFile".into(), Value::Native(natives::process_exec_file));
                let argv: Vec<Value> = tishlang_core::process_argv()
                    .into_iter()
                    .map(|s| Value::String(s.into()))
                    .collect();
                process_obj.insert("argv".into(), Value::Array(Rc::new(RefCell::new(argv))));
                let env_obj: PropMap = std::env::vars()
                    .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
                    .collect();
                process_obj.insert("env".into(), Value::object(env_obj));
                s.set("process".into(), Value::object(process_obj), true);
            }

            let mut object = PropMap::with_capacity(5);
            object.insert("keys".into(), Value::Native(Self::object_keys));
            object.insert("values".into(), Value::Native(Self::object_values));
            object.insert("entries".into(), Value::Native(Self::object_entries));
            object.insert("assign".into(), Value::Native(Self::object_assign));
            object.insert(
                "fromEntries".into(),
                Value::Native(Self::object_from_entries),
            );
            s.set(
                "Object".into(),
                Value::object(object),
                true,
            );

            let mut array_obj = PropMap::with_capacity(3);
            array_obj.insert("isArray".into(), Value::Native(natives::array_is_array));
            // `Array(n)` and `new Array(n)` constructor (issue #72).
            array_obj.insert("__call".into(), Value::Native(natives::array_construct));
            array_obj.insert("__construct".into(), Value::Native(natives::array_construct));
            s.set(
                "Array".into(),
                Value::object(array_obj),
                true,
            );

            // Error constructors (issue #60): callable + constructable via __call/__construct.
            for (name, ctor) in [
                ("Error", natives::error_construct as fn(&[Value]) -> Result<Value, String>),
                ("TypeError", natives::type_error_construct),
                ("RangeError", natives::range_error_construct),
                ("SyntaxError", natives::syntax_error_construct),
            ] {
                let mut err_obj = PropMap::with_capacity(2);
                err_obj.insert("__call".into(), Value::Native(ctor));
                err_obj.insert("__construct".into(), Value::Native(ctor));
                s.set(name.into(), Value::object(err_obj), true);
            }

            let mut string_obj = PropMap::with_capacity(2);
            string_obj.insert(
                "fromCharCode".into(),
                Value::Native(natives::string_from_char_code),
            );
            // `String(value)` callable: dispatched via `__call` in `call_func`, like `Symbol`.
            string_obj.insert("__call".into(), Value::Native(natives::string_convert));
            s.set(
                "String".into(),
                Value::object(string_obj),
                true,
            );

            // `Number(value)` coercion as a callable global (issue #36).
            let mut number_obj = PropMap::with_capacity(1);
            number_obj.insert("__call".into(), Value::Native(natives::number_convert));
            s.set("Number".into(), Value::object(number_obj), true);

            s.set(
                "Date".into(),
                crate::value_convert::core_to_eval(
                    tishlang_builtins::date::date_constructor_value(),
                ),
                true,
            );
            s.set(
                "Set".into(),
                crate::value_convert::core_to_eval(
                    tishlang_builtins::collections::set_constructor_value(),
                ),
                true,
            );
            s.set(
                "Map".into(),
                crate::value_convert::core_to_eval(
                    tishlang_builtins::collections::map_constructor_value(),
                ),
                true,
            );

            s.set(
                "Symbol".into(),
                crate::value_convert::core_to_eval(tishlang_builtins::symbol::symbol_object()),
                true,
            );
            for (name, ctor) in [
                (
                    "Float64Array",
                    tishlang_builtins::typedarrays::float64_array_constructor_value
                        as fn() -> tishlang_core::Value,
                ),
                ("Float32Array", tishlang_builtins::typedarrays::float32_array_constructor_value),
                ("Int8Array", tishlang_builtins::typedarrays::int8_array_constructor_value),
                ("Uint8Array", tishlang_builtins::typedarrays::uint8_array_constructor_value),
                (
                    "Uint8ClampedArray",
                    tishlang_builtins::typedarrays::uint8_clamped_array_constructor_value,
                ),
                ("Int16Array", tishlang_builtins::typedarrays::int16_array_constructor_value),
                ("Uint16Array", tishlang_builtins::typedarrays::uint16_array_constructor_value),
                ("Int32Array", tishlang_builtins::typedarrays::int32_array_constructor_value),
                ("Uint32Array", tishlang_builtins::typedarrays::uint32_array_constructor_value),
            ] {
                s.set(name.into(), crate::value_convert::core_to_eval(ctor()), true);
            }
            s.set(
                "AudioContext".into(),
                crate::value_convert::core_to_eval(
                    tishlang_builtins::construct::audio_context_constructor_value(),
                ),
                true,
            );

            #[cfg(feature = "regex")]
            {
                s.set(
                    "RegExp".into(),
                    Value::Native(Self::regexp_constructor_native),
                    true,
                );
            }

            // fs, process: prefer `import { x } from 'tish:fs'` etc.
            #[cfg(feature = "timers")]
            {
                s.set(
                    "setTimeout".into(),
                    Value::TimerBuiltin(Arc::from("setTimeout")),
                    true,
                );
                s.set(
                    "setInterval".into(),
                    Value::TimerBuiltin(Arc::from("setInterval")),
                    true,
                );
                s.set(
                    "clearTimeout".into(),
                    Value::Native(Self::clear_timeout_native),
                    true,
                );
                s.set(
                    "clearInterval".into(),
                    Value::Native(Self::clear_interval_native),
                    true,
                );
            }
            #[cfg(feature = "http")]
            {
                s.set("fetch".into(), Value::Native(Self::fetch_native), true);
                s.set(
                    "fetchAll".into(),
                    Value::Native(Self::fetch_all_native),
                    true,
                );
                s.set("Promise".into(), Value::PromiseConstructor, true);
                s.set("serve".into(), Value::Serve, true);
            }
        }
        Self {
            scope,
            module_cache: Rc::new(RefCell::new(HashMap::new())),
            current_dir: RefCell::new(None),
            virtual_builtins: Rc::new(RefCell::new(HashMap::new())),
            string_builder: Rc::new(StringBuilderState::default()),
            call_depth: Rc::new(std::cell::Cell::new(0)),
            max_call_depth: env_max_call_depth(),
        }
    }

    /// Create an evaluator with extra native modules (e.g. Polars) registered.
    pub fn with_modules(modules: &[&dyn crate::TishNativeModule]) -> Self {
        let eval = Self::new();
        {
            let mut s = eval.scope.borrow_mut();
            for module in modules {
                for (name, value) in module.register() {
                    s.set(name, value, true);
                }
            }
        }
        {
            let mut vb = eval.virtual_builtins.borrow_mut();
            for module in modules {
                for (spec, value) in module.virtual_builtin_modules() {
                    vb.insert(Arc::from(spec), value);
                }
            }
        }
        eval
    }

    pub fn set_current_dir(&self, dir: Option<&Path>) {
        *self.current_dir.borrow_mut() = dir.map(PathBuf::from);
    }

    pub fn eval_program(&mut self, program: &tishlang_ast::Program) -> Result<Value, String> {
        let mut last = Value::Null;
        for stmt in &program.statements {
            last = self.eval_statement(stmt).map_err(|e| e.to_string())?;
        }
        // Flush any still-buffered string accumulator so the variable's slot is correct for any
        // post-run observation (timers, REPL, embedders reading scope state).
        self.flush_pending();
        Ok(last)
    }

    fn eval_statement(&mut self, stmt: &Statement) -> Result<Value, EvalError> {
        match stmt {
            Statement::Block { statements, .. } => {
                let scope = Scope::child(Rc::clone(&self.scope));
                let prev = std::mem::replace(&mut self.scope, scope);
                let mut last = Value::Null;
                for s in statements {
                    last = self.eval_statement(s)?;
                }
                self.scope = prev;
                Ok(last)
            }
            // Comma-declarators: a transparent group — evaluate each declarator in
            // the *current* scope (no child scope).
            Statement::Multi { statements, .. } => {
                let mut last = Value::Null;
                for s in statements {
                    last = self.eval_statement(s)?;
                }
                Ok(last)
            }
            Statement::VarDecl {
                name,
                mutable,
                init,
                ..
            } => {
                let value = init
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .transpose()?
                    .unwrap_or(Value::Null);
                self.scope
                    .borrow_mut()
                    .set(Arc::clone(name), value, *mutable);
                Ok(Value::Null)
            }
            Statement::VarDeclDestructure {
                pattern,
                mutable,
                init,
                ..
            } => {
                let value = self.eval_expr(init)?;
                self.bind_destruct_pattern(pattern, &value, *mutable)?;
                Ok(Value::Null)
            }
            Statement::ExprStmt { expr, .. } => {
                // Statement position: route through the path that keeps statement-position
                // `acc += x` O(1) (no result materialization) while preserving values otherwise.
                self.eval_expr_discard(expr)
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.eval_expr(cond)?;
                if c.is_truthy() {
                    self.eval_statement(then_branch)
                } else if let Some(eb) = else_branch {
                    self.eval_statement(eb)
                } else {
                    Ok(Value::Null)
                }
            }
            Statement::While { cond, body, .. } => {
                loop {
                    if !self.eval_expr(cond)?.is_truthy() {
                        break;
                    }
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }
            Statement::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                let iter_val = self.eval_expr(iterable)?;
                let elements = match &iter_val {
                    crate::value::Value::Array(arr) => {
                        arr.borrow().iter().cloned().collect::<Vec<_>>()
                    }
                    crate::value::Value::String(s) => s
                        .chars()
                        .map(|c| crate::value::Value::String(Arc::from(c.to_string())))
                        .collect::<Vec<_>>(),
                    // Iterator protocol: an object with a callable `next()` returning
                    // `{ value, done }` — e.g. a Map/Set iterator from `.values()` /
                    // `.keys()` / `.entries()`. Drain it ONCE (draining advances the
                    // iterator's shared position, so it must not be re-run).
                    _ => match self.drain_eval_iterator(&iter_val) {
                        Some(elems) => elems,
                        None => {
                            return Err(EvalError::Error(format!(
                                "for-of requires iterable (array, string, or iterator), got {}",
                                iter_val
                            )));
                        }
                    },
                };
                // Each element gets a FRESH per-iteration binding (ES6 `for (let v of …)`), so a
                // closure created in the body captures that element, not the last one.
                let outer = Rc::clone(&self.scope);
                let mut ret = Ok(Value::Null);
                for elem in elements {
                    let iter_env = Scope::child(Rc::clone(&outer));
                    iter_env.borrow_mut().set(Arc::clone(name), elem, true);
                    self.scope = Rc::clone(&iter_env);
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => continue,
                        Err(e) => {
                            ret = Err(e);
                            break;
                        }
                    }
                }
                self.scope = outer;
                ret
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                // `let`/`const` declared in `init` get a FRESH per-iteration binding (ES6), so a
                // closure created in the body captures THAT iteration's value, not the final one.
                // The canonical values live in `loop_env`; each iteration's body runs in a fresh
                // `iter_env` copy, and mutations are copied back for the next test/update.
                let outer = Rc::clone(&self.scope);
                let loop_env = Scope::child(Rc::clone(&outer));
                self.scope = Rc::clone(&loop_env);
                if let Some(i) = init {
                    if let Err(e) = self.eval_statement(i) {
                        self.scope = outer;
                        return Err(e);
                    }
                }
                let per_iter: Vec<Arc<str>> = loop_env.borrow().vars.keys().cloned().collect();
                let copy_vars = |from: &ScopeRef, to: &ScopeRef, names: &[Arc<str>]| {
                    let src = from.borrow();
                    let mut dst = to.borrow_mut();
                    for n in names {
                        if let Some(v) = src.vars.get(n.as_ref()) {
                            dst.set(Arc::clone(n), v.clone(), true);
                        }
                    }
                };
                let mut ret = Ok(Value::Null);
                loop {
                    self.scope = Rc::clone(&loop_env);
                    let cond_ok = match cond.as_ref() {
                        Some(c) => match self.eval_expr(c) {
                            Ok(v) => v.is_truthy(),
                            Err(e) => {
                                ret = Err(e);
                                break;
                            }
                        },
                        None => true,
                    };
                    if !cond_ok {
                        break;
                    }
                    let iter_env = if per_iter.is_empty() {
                        Rc::clone(&loop_env)
                    } else {
                        let e = Scope::child(Rc::clone(&outer));
                        copy_vars(&loop_env, &e, &per_iter);
                        e
                    };
                    self.scope = Rc::clone(&iter_env);
                    let flow = self.eval_statement(body);
                    if !per_iter.is_empty() {
                        copy_vars(&iter_env, &loop_env, &per_iter);
                    }
                    match flow {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => {}
                        Err(e) => {
                            ret = Err(e);
                            break;
                        }
                    }
                    self.scope = Rc::clone(&loop_env);
                    if let Some(u) = update {
                        if let Err(e) = self.eval_expr(u) {
                            ret = Err(e);
                            break;
                        }
                    }
                }
                self.scope = outer;
                ret
            }
            Statement::Return { value, .. } => {
                let v = value
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .transpose()?
                    .unwrap_or(Value::Null);
                Err(EvalError::Return(v))
            }
            Statement::Break { .. } => Err(EvalError::Break),
            Statement::Continue { .. } => Err(EvalError::Continue),
            Statement::FunDecl {
                name,
                params,
                rest_param,
                body,
                ..
            } => {
                let formals: Arc<[FunParam]> = Arc::from(params.clone());
                let rest_param_name = rest_param.as_ref().map(|p| Arc::clone(&p.name));
                let body = Arc::new(body.as_ref().clone());
                let func = Value::Function {
                    formals,
                    rest_param: rest_param_name,
                    body,
                    // Capture the defining scope. It's the SAME Rc we insert into below, so the
                    // function sees itself → recursion works.
                    env: Rc::clone(&self.scope),
                };
                self.scope.borrow_mut().set(Arc::clone(name), func, true);
                Ok(Value::Null)
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                let v = self.eval_expr(expr)?;
                let mut matched = false;
                for (case_expr, body) in cases {
                    if let Some(ce) = case_expr {
                        let cv = self.eval_expr(ce)?;
                        if v.strict_eq(&cv) {
                            matched = true;
                            let scope = Scope::child(Rc::clone(&self.scope));
                            let prev = std::mem::replace(&mut self.scope, scope);
                            for s in body {
                                match self.eval_statement(s) {
                                    Ok(_) => {}
                                    Err(EvalError::Break) => {
                                        self.scope = prev;
                                        return Ok(Value::Null);
                                    }
                                    Err(e) => {
                                        self.scope = prev;
                                        return Err(e);
                                    }
                                }
                            }
                            self.scope = prev;
                            break;
                        }
                    }
                }
                if !matched {
                    if let Some(body) = default_body {
                        let scope = Scope::child(Rc::clone(&self.scope));
                        let prev = std::mem::replace(&mut self.scope, scope);
                        for s in body {
                            match self.eval_statement(s) {
                                Ok(_) => {}
                                Err(EvalError::Break) => break,
                                Err(e) => {
                                    self.scope = prev;
                                    return Err(e);
                                }
                            }
                        }
                        self.scope = prev;
                    }
                }
                Ok(Value::Null)
            }
            Statement::DoWhile { body, cond, .. } => {
                loop {
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => {
                            if !self.eval_expr(cond)?.is_truthy() {
                                break;
                            }
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                    if !self.eval_expr(cond)?.is_truthy() {
                        break;
                    }
                }
                Ok(Value::Null)
            }
            Statement::Throw { value, .. } => {
                let v = self.eval_expr(value)?;
                Err(EvalError::Throw(v))
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                finally_body,
                ..
            } => {
                let try_result = self.eval_statement(body);

                // Both a user `throw` and a runtime error (`null.foo()`, "not a function", …)
                // are catchable (issue #60); a runtime error is boxed as a `{ name, message }`
                // object so `catch (e) { e.message }` works. Break/Continue/Return propagate.
                let caught: Option<Value> = match &try_result {
                    Err(EvalError::Throw(v)) => Some(v.clone()),
                    Err(EvalError::Error(msg)) => {
                        let mut m = crate::value::PropMap::with_capacity(2);
                        m.insert("name".into(), Value::String("TypeError".into()));
                        m.insert("message".into(), Value::String(msg.as_str().into()));
                        Some(Value::object(m))
                    }
                    _ => None,
                };

                let result = match caught {
                    Some(thrown) => {
                        if let Some(catch_stmt) = catch_body {
                            if let Some(param) = catch_param {
                                let scope = Scope::child(Rc::clone(&self.scope));
                                let prev = std::mem::replace(&mut self.scope, Rc::clone(&scope));
                                scope.borrow_mut().set(Arc::clone(param), thrown, true);
                                let res = self.eval_statement(catch_stmt);
                                self.scope = prev;
                                res
                            } else {
                                self.eval_statement(catch_stmt)
                            }
                        } else {
                            // No catch clause — re-raise the original error after `finally`.
                            try_result
                        }
                    }
                    None => try_result,
                };

                if let Some(finally_stmt) = finally_body {
                    // KNOWN BUG (shared with the VM/compiled backends): a throw/return/
                    // break/continue inside `finally` should supersede the try/catch
                    // outcome (JS completion semantics) but is swallowed here. Fixing it
                    // in the interp alone (`?`) breaks interp==vm parity because the VM has
                    // the same bug, and the VM fix is a bytecode-compiler-level
                    // finally-completion change. Deferred as a coordinated cross-backend fix.
                    let _ = self.eval_statement(finally_stmt);
                }

                result
            }
            Statement::Import {
                specifiers, from, ..
            } => {
                let exports_val = self.load_module(from)?;
                let exports = match &exports_val {
                    Value::Object(m) => m.borrow().clone(),
                    _ => {
                        return Err(EvalError::Error(
                            "Module exports must be object".to_string(),
                        ))
                    }
                };
                let mut scope = self.scope.borrow_mut();
                for spec in specifiers {
                    match spec {
                        ImportSpecifier::Named { name, alias, .. } => {
                            let v = exports.strings.get(name.as_ref()).ok_or_else(|| {
                                EvalError::Error(format!("Module does not export '{}'", name))
                            })?;
                            let bind = alias.as_deref().unwrap_or(name.as_ref());
                            scope.set(Arc::from(bind), v.clone(), false);
                        }
                        ImportSpecifier::Namespace { name, .. } => {
                            scope.set(Arc::clone(name), exports_val.clone(), false);
                        }
                        ImportSpecifier::Default { name, .. } => {
                            let v = exports.strings.get("default").ok_or_else(|| {
                                EvalError::Error("Module does not have default export".to_string())
                            })?;
                            scope.set(Arc::clone(name), v.clone(), false);
                        }
                    }
                }
                Ok(Value::Null)
            }
            Statement::Export { declaration, .. } => {
                match declaration.as_ref() {
                    ExportDeclaration::Named(s) => {
                        let _ = self.eval_statement(s);
                    }
                    ExportDeclaration::Default(e) => {
                        let v = self.eval_expr(e)?;
                        self.scope.borrow_mut().set(Arc::from("default"), v, false);
                    }
                    // #305: re-export in the running program — load the dep and bind the names locally.
                    ExportDeclaration::ReExport {
                        specifiers,
                        all,
                        from,
                        ..
                    } => {
                        let dep_val = self.load_module(from.as_ref())?;
                        let dep = match &dep_val {
                            Value::Object(m) => m.borrow().clone(),
                            _ => return Err(EvalError::Error("Module exports must be object".to_string())),
                        };
                        if *all {
                            for (k, v) in dep.strings.iter() {
                                self.scope.borrow_mut().set(Arc::from(k.as_ref()), v.clone(), false);
                            }
                        }
                        for spec in specifiers {
                            if let ImportSpecifier::Named { name, alias, .. } = spec {
                                let v = dep.strings.get(name.as_ref()).ok_or_else(|| {
                                    EvalError::Error(format!("Module does not export '{}'", name))
                                })?;
                                let bind = alias.as_deref().unwrap_or(name.as_ref());
                                self.scope.borrow_mut().set(Arc::from(bind), v.clone(), false);
                            }
                        }
                    }
                }
                Ok(Value::Null)
            }
            Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => Ok(Value::Null),
        }
    }

    /// Load and evaluate a module, returning its exports object. Uses cache.
    fn load_module(&mut self, from: &str) -> Result<Value, EvalError> {
        if from.starts_with("cargo:") {
            return Err(EvalError::Error(
                "cargo:… imports are only supported by `tish build` with the Rust native backend."
                    .into(),
            ));
        }
        if from.starts_with("tish:") {
            return self.load_builtin_module(from);
        }
        // Scoped native modules (e.g. `@tishlang/waterui`) registered via `TishNativeModule::virtual_builtin_modules`.
        if self.virtual_builtins.borrow().get(from).is_some() {
            return self.load_builtin_module(from);
        }
        let dir = self.current_dir.borrow().clone().ok_or_else(|| {
            EvalError::Error(
                "Cannot resolve imports: no current file directory (use run_file)".to_string(),
            )
        })?;
        let path = Self::resolve_import_path(from, &dir)?;
        let path = path
            .canonicalize()
            .map_err(|e| EvalError::Error(format!("Cannot resolve import '{}': {}", from, e)))?;
        {
            let cache = self.module_cache.borrow();
            if let Some(m) = cache.get(&path) {
                return Ok(m.clone());
            }
        }
        let source = std::fs::read_to_string(&path)
            .map_err(|e| EvalError::Error(format!("Cannot read {}: {}", path.display(), e)))?;
        let program = tishlang_parser::parse(&source)
            .map_err(|e| EvalError::Error(format!("Parse error in {}: {}", path.display(), e)))?;
        let module_scope = Scope::child(Rc::clone(&self.scope));
        let prev_scope = std::mem::replace(&mut self.scope, Rc::clone(&module_scope));
        let parent_dir = self.current_dir.borrow().clone();
        let module_dir = path.parent().map(PathBuf::from);
        *self.current_dir.borrow_mut() = module_dir;
        let mut export_names: Vec<String> = Vec::new();
        for stmt in &program.statements {
            if let Statement::Export { declaration, .. } = stmt {
                match declaration.as_ref() {
                    ExportDeclaration::Named(s) => {
                        let _ = self.eval_statement(s);
                        if let Statement::VarDecl { name, .. } | Statement::FunDecl { name, .. } =
                            s.as_ref()
                        {
                            export_names.push(name.to_string());
                        }
                    }
                    ExportDeclaration::Default(e) => {
                        let v = self.eval_expr(e)?;
                        self.scope.borrow_mut().set(Arc::from("default"), v, false);
                        export_names.push("default".to_string());
                    }
                    // #305: re-export — load the dep, pull the requested (or all) exports into this
                    // module's scope, and re-expose them in `export_names`.
                    ExportDeclaration::ReExport {
                        specifiers,
                        all,
                        from,
                        ..
                    } => {
                        let dep_val = self.load_module(from.as_ref())?;
                        let dep = match &dep_val {
                            Value::Object(m) => m.borrow().clone(),
                            _ => return Err(EvalError::Error("Module exports must be object".to_string())),
                        };
                        if *all {
                            for (k, v) in dep.strings.iter() {
                                self.scope.borrow_mut().set(Arc::from(k.as_ref()), v.clone(), false);
                                export_names.push(k.to_string());
                            }
                        }
                        for spec in specifiers {
                            if let ImportSpecifier::Named { name, alias, .. } = spec {
                                let v = dep.strings.get(name.as_ref()).ok_or_else(|| {
                                    EvalError::Error(format!("Module does not export '{}'", name))
                                })?;
                                let bind = alias.as_deref().unwrap_or(name.as_ref());
                                self.scope.borrow_mut().set(Arc::from(bind), v.clone(), false);
                                export_names.push(bind.to_string());
                            }
                        }
                    }
                }
            } else {
                let _ = self.eval_statement(stmt);
            }
        }
        let mut exports: PropMap = PropMap::default();
        for name in export_names {
            if let Some(v) = module_scope.borrow().get(&name) {
                exports.insert(Arc::from(name.as_str()), v);
            }
        }
        *self.current_dir.borrow_mut() = parent_dir;
        self.scope = prev_scope;
        let exports_val = Value::object(exports);
        self.module_cache
            .borrow_mut()
            .insert(path, exports_val.clone());
        Ok(exports_val)
    }

    fn resolve_import_path(from: &str, dir: &Path) -> Result<PathBuf, EvalError> {
        if !from.starts_with("./") && !from.starts_with("../") {
            return Err(EvalError::Error(format!(
                "Only relative imports supported (./ or ../), got: {}",
                from
            )));
        }
        let base = dir.join(from);
        let path = if base.extension().is_none() {
            let with_ext = base.with_extension("tish");
            if with_ext.exists() {
                with_ext
            } else {
                base
            }
        } else {
            base
        };
        Ok(path)
    }

    /// Load built-in module (tish:fs, tish:http, tish:process, …) or a virtual module from native crates.
    fn load_builtin_module(&self, spec: &str) -> Result<Value, EvalError> {
        if spec.starts_with("cargo:") {
            return Err(EvalError::Error(
                "cargo:… imports are only supported when compiling with `tish build` and the Rust native backend. They link Cargo crates via package.json tish.rustDependencies and a generated native wrapper — not the interpreter or VM.".into(),
            ));
        }
        if let Some(v) = self.virtual_builtins.borrow().get(spec) {
            return Ok(v.clone());
        }
        match spec {
            "tish:fs" => {
                #[cfg(feature = "fs")]
                {
                    let mut exports: PropMap = PropMap::default();
                    exports.insert("readFile".into(), Value::Native(natives::read_file));
                    exports.insert("writeFile".into(), Value::Native(natives::write_file));
                    exports.insert("fileExists".into(), Value::Native(natives::file_exists));
                    exports.insert("isDir".into(), Value::Native(natives::is_dir));
                    exports.insert("readDir".into(), Value::Native(natives::read_dir));
                    exports.insert(
                        "readFileBytes".into(),
                        Value::Native(natives::read_file_bytes),
                    );
                    exports.insert("mkdir".into(), Value::Native(natives::mkdir));
                    Ok(Value::object(exports))
                }
                #[cfg(not(feature = "fs"))]
                {
                    return Err(EvalError::Error(
                        "tish:fs requires the fs feature. Rebuild with: cargo build -p tishlang --features fs".into(),
                    ));
                }
            }
            "tish:http" => {
                #[cfg(feature = "http")]
                {
                    let mut exports: PropMap = PropMap::default();
                    exports.insert("fetch".into(), Value::Native(Self::fetch_native));
                    exports.insert("fetchAll".into(), Value::Native(Self::fetch_all_native));
                    exports.insert("serve".into(), Value::Serve);
                    exports.insert("Promise".into(), Value::PromiseConstructor);
                    Ok(Value::object(exports))
                }
                #[cfg(not(feature = "http"))]
                {
                    return Err(EvalError::Error(
                        "tish:http requires the http feature. Rebuild with: cargo build -p tishlang --features http".into(),
                    ));
                }
            }
            "tish:timers" => {
                #[cfg(feature = "timers")]
                {
                    let mut exports: PropMap = PropMap::default();
                    exports.insert(
                        "setTimeout".into(),
                        Value::TimerBuiltin(Arc::from("setTimeout")),
                    );
                    exports.insert(
                        "setInterval".into(),
                        Value::TimerBuiltin(Arc::from("setInterval")),
                    );
                    exports.insert(
                        "clearTimeout".into(),
                        Value::Native(Self::clear_timeout_native),
                    );
                    exports.insert(
                        "clearInterval".into(),
                        Value::Native(Self::clear_interval_native),
                    );
                    Ok(Value::object(exports))
                }
                #[cfg(not(feature = "timers"))]
                {
                    return Err(EvalError::Error(
                        "tish:timers requires the timers feature. Rebuild with: cargo build -p tishlang --features timers".into(),
                    ));
                }
            }
            "tish:ws" => {
                #[cfg(feature = "ws")]
                {
                    let mut exports: PropMap = PropMap::default();
                    exports.insert(
                        "WebSocket".into(),
                        Value::Native(Self::ws_web_socket_native),
                    );
                    exports.insert("Server".into(), Value::Native(Self::ws_server_native));
                    exports.insert("wsSend".into(), Value::Native(Self::ws_send_native));
                    exports.insert(
                        "wsBroadcast".into(),
                        Value::Native(Self::ws_broadcast_native),
                    );
                    Ok(Value::object(exports))
                }
                #[cfg(not(feature = "ws"))]
                {
                    return Err(EvalError::Error(
                        "tish:ws requires the ws feature. Rebuild with: cargo build -p tishlang --features ws".into(),
                    ));
                }
            }
            "tish:tty" => {
                #[cfg(feature = "tty")]
                {
                    let mut exports: PropMap = PropMap::default();
                    exports.insert("size".into(), Value::Native(natives::tty_size));
                    exports.insert("isTTY".into(), Value::Native(natives::tty_is_tty));
                    exports.insert("setRawMode".into(), Value::Native(natives::tty_set_raw_mode));
                    exports.insert(
                        "enterAltScreen".into(),
                        Value::Native(natives::tty_enter_alt_screen),
                    );
                    exports.insert(
                        "leaveAltScreen".into(),
                        Value::Native(natives::tty_leave_alt_screen),
                    );
                    exports.insert("read".into(), Value::Native(natives::tty_read));
                    exports.insert("readLine".into(), Value::Native(natives::tty_read_line));
                    Ok(Value::object(exports))
                }
                #[cfg(not(feature = "tty"))]
                {
                    return Err(EvalError::Error(
                        "tish:tty requires the tty feature. Rebuild with: cargo build -p tishlang --features tty".into(),
                    ));
                }
            }
            "tish:process" => {
                #[cfg(feature = "process")]
                {
                    let mut exports: PropMap = PropMap::default();
                    exports.insert("exit".into(), Value::Native(natives::process_exit));
                    exports.insert("cwd".into(), Value::Native(natives::process_cwd));
                    exports.insert("exec".into(), Value::Native(natives::process_exec));
                    exports.insert("execFile".into(), Value::Native(natives::process_exec_file));
                    let argv: Vec<Value> = tishlang_core::process_argv()
                        .into_iter()
                        .map(|s| Value::String(s.into()))
                        .collect();
                    exports.insert(
                        "argv".into(),
                        Value::Array(Rc::new(RefCell::new(argv.clone()))),
                    );
                    let env_obj: PropMap = std::env::vars()
                        .map(|(key, value)| (Arc::from(key.as_str()), Value::String(value.into())))
                        .collect();
                    exports.insert(
                        "env".into(),
                        Value::object(env_obj.clone()),
                    );
                    let mut process_obj = PropMap::default();
                    process_obj.insert("exit".into(), Value::Native(natives::process_exit));
                    process_obj.insert("cwd".into(), Value::Native(natives::process_cwd));
                    process_obj.insert("exec".into(), Value::Native(natives::process_exec));
                    process_obj.insert("execFile".into(), Value::Native(natives::process_exec_file));
                    process_obj.insert("argv".into(), Value::Array(Rc::new(RefCell::new(argv))));
                    process_obj.insert("env".into(), Value::object(env_obj));
                    exports.insert(
                        "process".into(),
                        Value::object(process_obj),
                    );
                    Ok(Value::object(exports))
                }
                #[cfg(not(feature = "process"))]
                {
                    return Err(EvalError::Error(
                        "tish:process requires the process feature. Rebuild with: cargo build -p tishlang --features process".into(),
                    ));
                }
            }
            _ => {
                Err(EvalError::Error(format!(
                    "Unknown built-in module: {}. Supported: tish:fs, tish:http, tish:timers, tish:process, tish:ws (plus any registered by native modules)",
                    spec
                )))
            }
        }
    }

    fn load_builtin_export(&self, spec: &str, export_name: &str) -> Result<Value, EvalError> {
        let module = self.load_builtin_module(spec)?;
        let exports = match &module {
            Value::Object(m) => m.borrow().clone(),
            _ => return Err(EvalError::Error("Built-in module must be object".into())),
        };
        exports
            .strings
            .get(export_name)
            .cloned()
            .ok_or_else(|| {
                EvalError::Error(format!("Module {} does not export '{}'", spec, export_name))
            })
    }

    // --- string-builder helpers (#186 / string_build): amortized O(1) `acc += x` ---

    /// Append a value to a builder buffer using the exact JS coercion of `eval_binop`'s `+` (string
    /// operands push their raw chars; everything else goes through `to_js_string`).
    fn append_value_to_buf(buf: &mut String, v: &Value) {
        match v {
            Value::String(s) => buf.push_str(s),
            other => buf.push_str(&other.to_js_string()),
        }
    }

    /// Walk the scope chain from the current scope and return the exact scope that owns `name`.
    fn find_var_scope(&self, name: &str) -> Option<Rc<std::cell::RefCell<Scope>>> {
        let mut cur = Rc::clone(&self.scope);
        loop {
            if cur.borrow().vars.contains_key(name) {
                return Some(cur);
            }
            let parent = cur.borrow().parent.clone();
            match parent {
                Some(p) => cur = p,
                None => return None,
            }
        }
    }

    /// Flush the active builder (if any) back into its owning scope slot, restoring the invariant
    /// that the slot holds the variable's true value. No-op when no builder is active.
    fn flush_pending(&self) {
        if !self.string_builder.active.get() {
            return;
        }
        if let Some(p) = self.string_builder.pending.borrow_mut().take() {
            p.scope
                .borrow_mut()
                .vars
                .insert(Arc::clone(&p.name), Value::String(p.buf.into()));
        }
        self.string_builder.active.set(false);
    }

    /// Flush the builder iff it is buffering `name` (which is about to be read). The `active` guard
    /// keeps this ~free on the hot identifier-read path when no builder exists.
    #[inline]
    fn flush_pending_for(&self, name: &str) {
        if !self.string_builder.active.get() {
            return;
        }
        let hit = self
            .string_builder
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|p| p.name.as_ref() == name);
        if hit {
            self.flush_pending();
        }
    }

    /// Discard the builder iff it is buffering `name` (which is about to be overwritten by a plain
    /// assignment) — avoids an unnecessary O(n) flush of a value that is about to be replaced.
    #[inline]
    fn discard_pending_for(&self, name: &str) {
        if !self.string_builder.active.get() {
            return;
        }
        let hit = self
            .string_builder
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|p| p.name.as_ref() == name);
        if hit {
            *self.string_builder.pending.borrow_mut() = None;
            self.string_builder.active.set(false);
        }
    }

    /// Try to handle `name += rhs` as an amortized-O(1) string append. Returns `true` if it was a
    /// string append (buffer updated); `false` if `name` is not a (mutable) string accumulator, so
    /// the caller falls back to the generic `+=`. Any builder for a DIFFERENT slot is flushed first;
    /// keying by the exact owning scope means a shadowing inner `name` never appends onto an outer
    /// `name`'s buffer.
    fn try_string_append(&self, name: &Arc<str>, rhs: &Value) -> bool {
        let owner = match self.find_var_scope(name) {
            Some(s) => s,
            None => return false, // undefined → let the generic path raise the error
        };
        // Continue an existing builder for this exact slot.
        if self.string_builder.active.get() {
            let same = self
                .string_builder
                .pending
                .borrow()
                .as_ref()
                .is_some_and(|p| p.name == *name && Rc::ptr_eq(&p.scope, &owner));
            if same {
                if let Some(p) = self.string_builder.pending.borrow_mut().as_mut() {
                    Self::append_value_to_buf(&mut p.buf, rhs);
                }
                return true;
            }
            // A different slot is buffered — flush it before starting a new one.
            self.flush_pending();
        }
        // Start a new builder only if the accumulator currently holds a mutable string.
        let start = {
            let owner_ref = owner.borrow();
            if owner_ref.consts.contains(name.as_ref()) {
                return false; // `const acc += x` must raise the same error as the generic path
            }
            match owner_ref.vars.get(name.as_ref()) {
                Some(Value::String(a)) => Some(a.clone()),
                _ => None, // non-string accumulator → numeric/other `+=`
            }
        };
        match start {
            Some(a) => {
                let mut buf = String::with_capacity(a.len() + 16);
                buf.push_str(&a);
                Self::append_value_to_buf(&mut buf, rhs);
                *self.string_builder.pending.borrow_mut() = Some(PendingAppend {
                    scope: owner,
                    name: Arc::clone(name),
                    buf,
                });
                self.string_builder.active.set(true);
                true
            }
            None => false,
        }
    }

    /// Evaluate `expr` in statement position. Special-cases `acc += rhs` so the string-builder never
    /// has to materialize the assignment's result value — the key to keeping the append O(1). In that
    /// one case the (discarded) statement value is `null`; every other expression returns its real
    /// value, preserving block/program last-value semantics.
    fn eval_expr_discard(&self, expr: &Expr) -> Result<Value, EvalError> {
        if let Expr::CompoundAssign {
            name,
            op: tishlang_ast::CompoundOp::Add,
            value,
            ..
        } = expr
        {
            // Evaluate rhs FIRST — it may read `name`, which flushes any active builder.
            let rhs = self.eval_expr(value)?;
            if self.try_string_append(name, &rhs) {
                // Statement-position result is discarded; returning null avoids the O(n) flatten.
                return Ok(Value::Null);
            }
            // Not a string accumulator: generic `+=` (numeric/other) — return its real value.
            self.flush_pending_for(name);
            let current = self
                .scope
                .borrow()
                .get(name.as_ref())
                .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
            let result = self
                .eval_binop(&current, BinOp::Add, &rhs)
                .map_err(EvalError::Error)?;
            match self.scope.borrow_mut().assign(name.as_ref(), result.clone()) {
                Ok(true) => Ok(result),
                Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                Err(e) => Err(EvalError::Error(e)),
            }
        } else {
            self.eval_expr(expr)
        }
    }

    fn eval_expr(&self, expr: &Expr) -> Result<Value, EvalError> {
        match expr {
            Expr::Literal { value, .. } => Ok(match value {
                Literal::Number(n) => Value::Number(*n),
                Literal::String(s) => Value::String(Arc::clone(s)),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Null => Value::Null,
            }),
            Expr::Ident { name, .. } => {
                // Flush any string-builder buffering this variable so the read sees the full string.
                self.flush_pending_for(name.as_ref());
                self.scope
                    .borrow()
                    .get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))
            }
            Expr::Binary {
                left,
                op,
                right,
                ..
            } => match op {
                // Short-circuit + value-returning && / || (JS, #240): evaluate the left; if it
                // already decides the result, return IT without evaluating (and running the side
                // effects of) the right. The result is always an operand value, never a coerced
                // boolean. The generic path below eagerly evaluated both operands and `eval_binop`
                // coerced And/Or to `Bool`, so `five() && 7` was `true` and `false && f()` still
                // ran `f()`.
                BinOp::And => {
                    let l = self.eval_expr(left)?;
                    if l.is_truthy() {
                        self.eval_expr(right)
                    } else {
                        Ok(l)
                    }
                }
                BinOp::Or => {
                    let l = self.eval_expr(left)?;
                    if l.is_truthy() {
                        Ok(l)
                    } else {
                        self.eval_expr(right)
                    }
                }
                _ => {
                    let l = self.eval_expr(left)?;
                    let r = self.eval_expr(right)?;
                    self.eval_binop(&l, *op, &r).map_err(EvalError::Error)
                }
            },
            Expr::Unary { op, operand, .. } => {
                let o = self.eval_expr(operand)?;
                self.eval_unary(*op, &o).map_err(EvalError::Error)
            }
            Expr::Call { callee, args, .. } => {
                // Check for built-in method calls on arrays/strings
                if let Expr::Member {
                    object,
                    prop: MemberProp::Name { name: method_name, .. },
                    ..
                } = callee.as_ref()
                {
                    let obj = self.eval_expr(object)?;
                    let arg_vals = self.eval_call_args(args)?;
                    
                    // Array methods
                    if let Value::Array(arr) = &obj {
                        match method_name.as_ref() {
                            "push" => {
                                let mut arr_mut = arr.borrow_mut();
                                for v in &arg_vals {
                                    arr_mut.push(v.clone());
                                }
                                return Ok(Value::Number(arr_mut.len() as f64));
                            }
                            "pop" => {
                                return Ok(arr.borrow_mut().pop().unwrap_or(Value::Null));
                            }
                            "shift" => {
                                let mut arr_mut = arr.borrow_mut();
                                if arr_mut.is_empty() {
                                    return Ok(Value::Null);
                                }
                                return Ok(arr_mut.remove(0));
                            }
                            "unshift" => {
                                let mut arr_mut = arr.borrow_mut();
                                for (i, v) in arg_vals.iter().enumerate() {
                                    arr_mut.insert(i, v.clone());
                                }
                                return Ok(Value::Number(arr_mut.len() as f64));
                            }
                            "indexOf" => {
                                let search = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    if v.strict_eq(&search) {
                                        return Ok(Value::Number(i as f64));
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "includes" => {
                                let search = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let len = arr_borrow.len() as i64;
                                let start = match arg_vals.get(1) {
                                    Some(Value::Number(n)) if *n >= 0.0 => (*n as i64).min(len).max(0) as usize,
                                    Some(Value::Number(n)) if *n < 0.0 => ((len + *n as i64).max(0)) as usize,
                                    _ => 0,
                                };
                                for v in arr_borrow.iter().skip(start) {
                                    // SameValueZero: NaN matches NaN (JS Array.includes, unlike
                                    // indexOf). #247
                                    if v.strict_eq(&search)
                                        || matches!((v, &search), (Value::Number(a), Value::Number(b)) if a.is_nan() && b.is_nan())
                                    {
                                        return Ok(Value::Bool(true));
                                    }
                                }
                                return Ok(Value::Bool(false));
                            }
                            "join" => {
                                let sep = match arg_vals.first() {
                                    Some(Value::String(s)) => s.to_string(),
                                    _ => ",".to_string(),
                                };
                                let arr_borrow = arr.borrow();
                                // JS join: null/undefined → "", else JS ToString (nested arrays
                                // recurse to a comma-join, objects → "[object Object]").
                                let parts: Vec<String> = arr_borrow
                                    .iter()
                                    .map(|v| match v {
                                        Value::Null => String::new(),
                                        other => other.to_js_string(),
                                    })
                                    .collect();
                                return Ok(Value::String(parts.join(&sep).into()));
                            }
                            "reverse" => {
                                arr.borrow_mut().reverse();
                                return Ok(obj.clone());
                            }
                            "fill" => {
                                // Array.prototype.fill(value, start?, end?) — in place (issue #76).
                                let value = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let mut arr_mut = arr.borrow_mut();
                                let len = arr_mut.len() as i64;
                                let norm = |v: Option<&Value>, dflt: usize| -> usize {
                                    match v {
                                        Some(Value::Number(n)) => {
                                            let n = *n as i64;
                                            if n < 0 { (len + n).max(0) as usize } else { (n as usize).min(len as usize) }
                                        }
                                        _ => dflt,
                                    }
                                };
                                let start = norm(arg_vals.get(1), 0);
                                let end = norm(arg_vals.get(2), len as usize);
                                let mut i = start;
                                while i < end && i < arr_mut.len() {
                                    arr_mut[i] = value.clone();
                                    i += 1;
                                }
                                drop(arr_mut);
                                return Ok(obj.clone());
                            }
                            "shuffle" => {
                                let mut v = arr.borrow().clone();
                                use rand::seq::SliceRandom;
                                v.shuffle(&mut rand::rng());
                                return Ok(Value::Array(Rc::new(RefCell::new(v))));
                            }
                            "sort" => {
                                let comparator = arg_vals.into_iter().next();
                                let mut arr_mut = arr.borrow_mut();

                                if let Some(cmp_fn) = comparator {
                                    // Check for fast path: (a, b) => a - b numeric ascending
                                    let is_numeric_asc = Self::is_numeric_sort_comparator(&cmp_fn, false);
                                    let is_numeric_desc = !is_numeric_asc && Self::is_numeric_sort_comparator(&cmp_fn, true);

                                    if is_numeric_asc {
                                        // Fast path: numeric ascending sort
                                        arr_mut.sort_by(|a, b| {
                                            let na = match a { Value::Number(n) => *n, _ => f64::NAN };
                                            let nb = match b { Value::Number(n) => *n, _ => f64::NAN };
                                            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                                        });
                                    } else if is_numeric_desc {
                                        // Fast path: numeric descending sort
                                        arr_mut.sort_by(|a, b| {
                                            let na = match a { Value::Number(n) => *n, _ => f64::NAN };
                                            let nb = match b { Value::Number(n) => *n, _ => f64::NAN };
                                            nb.partial_cmp(&na).unwrap_or(std::cmp::Ordering::Equal)
                                        });
                                    } else {
                                        // General case: use comparator function with optimized scope reuse
                                        let len = arr_mut.len();
                                        let mut indices: Vec<usize> = (0..len).collect();
                                        let arr_values: Vec<Value> = std::mem::take(&mut *arr_mut);
                                        // A `throw` from the comparator must propagate (be catchable) and
                                        // must NOT corrupt the array. Previously the `_ => Equal` arm
                                        // swallowed the `Err` and wrote a bogus reordering back; now we
                                        // capture the throw, stop comparing, and restore the original order
                                        // — matching the vm/native backends and node.
                                        let mut pending: Option<EvalError> = None;

                                        if let Some((scope, params, body)) = self.create_callback_scope(&cmp_fn) {
                                            indices.sort_by(|&i, &j| {
                                                if pending.is_some() {
                                                    return std::cmp::Ordering::Equal;
                                                }
                                                match self.call_with_scope(&scope, &params, &body, &[arr_values[i].clone(), arr_values[j].clone()]) {
                                                    Ok(Value::Number(n)) if n < 0.0 => std::cmp::Ordering::Less,
                                                    Ok(Value::Number(n)) if n > 0.0 => std::cmp::Ordering::Greater,
                                                    Ok(_) => std::cmp::Ordering::Equal,
                                                    Err(e) => { pending = Some(e); std::cmp::Ordering::Equal }
                                                }
                                            });
                                        } else {
                                            indices.sort_by(|&i, &j| {
                                                if pending.is_some() {
                                                    return std::cmp::Ordering::Equal;
                                                }
                                                match self.call_func(&cmp_fn, &[arr_values[i].clone(), arr_values[j].clone()]) {
                                                    Ok(Value::Number(n)) if n < 0.0 => std::cmp::Ordering::Less,
                                                    Ok(Value::Number(n)) if n > 0.0 => std::cmp::Ordering::Greater,
                                                    Ok(_) => std::cmp::Ordering::Equal,
                                                    Err(e) => { pending = Some(e); std::cmp::Ordering::Equal }
                                                }
                                            });
                                        }

                                        if let Some(e) = pending {
                                            // Comparator threw: leave the array untouched and re-raise.
                                            *arr_mut = arr_values;
                                            drop(arr_mut);
                                            return Err(e);
                                        }
                                        *arr_mut = indices.into_iter().map(|i| arr_values[i].clone()).collect();
                                    }
                                } else {
                                    // Default string sort - precompute strings once
                                    let mut pairs: Vec<(String, usize)> = arr_mut
                                        .iter()
                                        .enumerate()
                                        .map(|(i, v)| (v.to_string(), i))
                                        .collect();
                                    pairs.sort_by(|a, b| a.0.cmp(&b.0));
                                    let arr_values: Vec<Value> = std::mem::take(&mut *arr_mut);
                                    *arr_mut = pairs.into_iter().map(|(_, i)| arr_values[i].clone()).collect();
                                }
                                drop(arr_mut);
                                return Ok(obj.clone());
                            }
                            "splice" => {
                                let mut arr_mut = arr.borrow_mut();
                                let len = arr_mut.len() as i64;
                                
                                let start = match arg_vals.first() {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => 0,
                                };
                                
                                let delete_count = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => (*n as i64).max(0) as usize,
                                    _ => (len as usize).saturating_sub(start),
                                };
                                
                                let actual_delete = delete_count.min(arr_mut.len().saturating_sub(start));
                                let removed: Vec<Value> = arr_mut.drain(start..start + actual_delete).collect();
                                
                                if arg_vals.len() > 2 {
                                    let items_to_insert: Vec<Value> = arg_vals[2..].to_vec();
                                    for (i, item) in items_to_insert.into_iter().enumerate() {
                                        arr_mut.insert(start + i, item);
                                    }
                                }
                                
                                return Ok(Value::Array(Rc::new(RefCell::new(removed))));
                            }
                            "slice" => {
                                let arr_borrow = arr.borrow();
                                let len = arr_borrow.len() as i64;
                                let start = match arg_vals.first() {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => 0,
                                };
                                let end = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => len as usize,
                                };
                                let sliced: Vec<Value> = if start < end {
                                    arr_borrow[start..end].to_vec()
                                } else {
                                    vec![]
                                };
                                return Ok(Value::Array(Rc::new(RefCell::new(sliced))));
                            }
                            "concat" => {
                                let mut result = arr.borrow().clone();
                                for v in &arg_vals {
                                    if let Value::Array(other) = v {
                                        result.extend(other.borrow().iter().cloned());
                                    } else {
                                        result.push(v.clone());
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "map" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                // #382-style snapshot: own the elements and DROP the array borrow before
                                // any callback runs, so a callback that mutates the array (via the `array`
                                // arg or a capture) can't RefCell-panic. `arr_value` is the JS 3rd arg.
                                let arr_borrow = arr.borrow().clone();
                                let arr_value = Value::Array(arr.clone());
                                let mut result = Vec::with_capacity(arr_borrow.len());
                                // Try fastest path: simple single-expression callbacks
                                let first_result = self.eval_simple_callback(&callback, &[arr_borrow.first().cloned().unwrap_or(Value::Null)]);
                                if first_result.is_some() {
                                    // Simple callback path - inline evaluation
                                    for v in arr_borrow.iter() {
                                        if let Some(r) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            result.push(r?);
                                        } else {
                                            // Shouldn't happen, but fall back
                                            result.push(self.call_func(&callback, &[v.clone()])?);
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    // Reusable scope path
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let mapped = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        result.push(mapped);
                                    }
                                } else {
                                    // Full call_func path
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let mapped = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        result.push(mapped);
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "flatMap" => {
                                // map + flatten one level (Array.prototype.flatMap). A callback `throw`
                                // propagates via `?` (catchable), matching vm/native/node — previously
                                // interp lacked flatMap entirely ("Not a function").
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                let mut result: Vec<Value> = Vec::with_capacity(arr_borrow.len());
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let mapped = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        match mapped {
                                            Value::Array(inner) => result.extend(inner.borrow().iter().cloned()),
                                            other => result.push(other),
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let mapped = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        match mapped {
                                            Value::Array(inner) => result.extend(inner.borrow().iter().cloned()),
                                            other => result.push(other),
                                        }
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "filter" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                let mut result = Vec::new();
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(keep) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if keep?.is_truthy() {
                                                result.push(v.clone());
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let keep = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if keep.is_truthy() {
                                            result.push(v.clone());
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let keep = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if keep.is_truthy() {
                                            result.push(v.clone());
                                        }
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "reduce" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 4th callback arg
                                let (mut acc, start_idx) = if arg_vals.len() > 1 {
                                    (arg_vals[1].clone(), 0)
                                } else if !arr_borrow.is_empty() {
                                    (arr_borrow[0].clone(), 1)
                                } else {
                                    return Err(EvalError::Error("Reduce of empty array with no initial value".to_string()));
                                };
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate().skip(start_idx) {
                                        acc = self.call_with_scope(&scope, &params, &body, &[acc, v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate().skip(start_idx) {
                                        acc = self.call_func(&callback, &[acc, v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                    }
                                }
                                return Ok(acc);
                            }
                            "find" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(found) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if found?.is_truthy() {
                                                return Ok(v.clone());
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if found.is_truthy() {
                                            return Ok(v.clone());
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if found.is_truthy() {
                                            return Ok(v.clone());
                                        }
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "findIndex" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if found.is_truthy() {
                                            return Ok(Value::Number(i as f64));
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if found.is_truthy() {
                                            return Ok(Value::Number(i as f64));
                                        }
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "findLast" => {
                                // Like find, from the end (#247). Callback gets (value, original index, array).
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                let scoped = self.create_callback_scope(&callback);
                                for i in (0..arr_borrow.len()).rev() {
                                    let v = arr_borrow[i].clone();
                                    let args = [v.clone(), Value::Number(i as f64), arr_value.clone()];
                                    let found = match &scoped {
                                        Some((scope, params, body)) => self.call_with_scope(scope, params, body, &args)?,
                                        None => self.call_func(&callback, &args)?,
                                    };
                                    if found.is_truthy() {
                                        return Ok(v);
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "findLastIndex" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                let scoped = self.create_callback_scope(&callback);
                                for i in (0..arr_borrow.len()).rev() {
                                    let args = [arr_borrow[i].clone(), Value::Number(i as f64), arr_value.clone()];
                                    let found = match &scoped {
                                        Some((scope, params, body)) => self.call_with_scope(scope, params, body, &args)?,
                                        None => self.call_func(&callback, &args)?,
                                    };
                                    if found.is_truthy() {
                                        return Ok(Value::Number(i as f64));
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "at" => {
                                // Negative index counts from the end; out of range → null (#247).
                                let i = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as i64,
                                    _ => 0,
                                };
                                let arr_borrow = arr.borrow();
                                let len = arr_borrow.len() as i64;
                                let idx = if i < 0 { len + i } else { i };
                                if idx >= 0 && idx < len {
                                    return Ok(arr_borrow[idx as usize].clone());
                                }
                                return Ok(Value::Null);
                            }
                            "forEach" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "some" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(result) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if result?.is_truthy() {
                                                return Ok(Value::Bool(true));
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if result.is_truthy() {
                                            return Ok(Value::Bool(true));
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if result.is_truthy() {
                                            return Ok(Value::Bool(true));
                                        }
                                    }
                                }
                                return Ok(Value::Bool(false));
                            }
                            "every" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow().clone(); // snapshot; drop borrow before callbacks
                                let arr_value = Value::Array(arr.clone()); // JS 3rd callback arg
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(result) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if !result?.is_truthy() {
                                                return Ok(Value::Bool(false));
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if !result.is_truthy() {
                                            return Ok(Value::Bool(false));
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_func(&callback, &[v.clone(), Value::Number(i as f64), arr_value.clone()])?;
                                        if !result.is_truthy() {
                                            return Ok(Value::Bool(false));
                                        }
                                    }
                                }
                                return Ok(Value::Bool(true));
                            }
                            "flat" => {
                                let depth = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 1,
                                };
                                fn flatten(arr: &[Value], depth: usize) -> Vec<Value> {
                                    let mut result = Vec::new();
                                    for v in arr {
                                        if depth > 0 {
                                            if let Value::Array(inner) = v {
                                                result.extend(flatten(&inner.borrow(), depth - 1));
                                                continue;
                                            }
                                        }
                                        result.push(v.clone());
                                    }
                                    result
                                }
                                let flattened = flatten(&arr.borrow(), depth);
                                return Ok(Value::Array(Rc::new(RefCell::new(flattened))));
                            }
                            _ => {}
                        }
                    }
                    
                    // String methods
                    if let Value::String(s) = &obj {
                        match method_name.as_ref() {
                            "indexOf" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Number(-1.0)),
                                };
                                let from_char = match arg_vals.get(1) {
                                    Some(Value::Number(n)) if *n >= 0.0 => {
                                        (*n as usize).min(s.chars().count())
                                    }
                                    _ => 0,
                                };
                                let byte_start: usize = s.chars().take(from_char).map(|c| c.len_utf8()).sum();
                                let found = s[byte_start..].find(search).map(|byte_pos| {
                                    let char_idx = from_char
                                        + s[byte_start..][..byte_pos].chars().count();
                                    char_idx as f64
                                });
                                return Ok(Value::Number(found.unwrap_or(-1.0)));
                            }
                            "lastIndexOf" => {
                                return Ok(Self::string_last_index_of_eval(&arg_vals, s));
                            }
                            "includes" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                let from_char = match arg_vals.get(1) {
                                    Some(Value::Number(n)) if *n >= 0.0 => (*n as usize).min(s.chars().count()),
                                    Some(Value::Number(n)) if *n < 0.0 => {
                                        let len = s.chars().count() as i64;
                                        ((len + *n as i64).max(0)) as usize
                                    }
                                    _ => 0,
                                };
                                let byte_start: usize = s.chars().take(from_char).map(|c| c.len_utf8()).sum();
                                return Ok(Value::Bool(s[byte_start..].contains(search)));
                            }
                            "slice" => {
                                let chars: Vec<char> = s.chars().collect();
                                let len = chars.len() as i64;
                                let start = match arg_vals.first() {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => 0,
                                };
                                let end = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => len as usize,
                                };
                                let sliced: String = if start < end {
                                    chars[start..end].iter().collect()
                                } else {
                                    String::new()
                                };
                                return Ok(Value::String(sliced.into()));
                            }
                            "substring" => {
                                let chars: Vec<char> = s.chars().collect();
                                let len = chars.len();
                                let start = match arg_vals.first() {
                                    Some(Value::Number(n)) => (*n as usize).min(len),
                                    _ => 0,
                                };
                                let end = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => (*n as usize).min(len),
                                    _ => len,
                                };
                                let (s, e) = (start.min(end), start.max(end));
                                return Ok(Value::String(chars[s..e].iter().collect::<String>().into()));
                            }
                            "split" => {
                                #[cfg(feature = "regex")]
                                if let Some(sep) = arg_vals.first() {
                                    let limit = arg_vals.get(1).and_then(|v| match v {
                                        Value::Number(n) => Some(*n as usize),
                                        _ => None,
                                    });
                                    return Ok(crate::regex::string_split(s, sep, limit));
                                }
                                #[cfg(not(feature = "regex"))]
                                {
                                    let sep = match arg_vals.first() {
                                        Some(Value::String(ss)) => ss.as_ref(),
                                        _ => return Ok(Value::Array(Rc::new(RefCell::new(vec![obj.clone()])))),
                                    };
                                    let parts: Vec<Value> = s.split(sep)
                                        .map(|p| Value::String(p.into()))
                                        .collect();
                                    return Ok(Value::Array(Rc::new(RefCell::new(parts))));
                                }
                                #[cfg(feature = "regex")]
                                return Ok(Value::Array(Rc::new(RefCell::new(vec![obj.clone()]))));
                            }
                            "trim" => {
                                return Ok(Value::String(s.trim().into()));
                            }
                            "toUpperCase" => {
                                return Ok(Value::String(s.to_uppercase().into()));
                            }
                            "toLowerCase" => {
                                return Ok(Value::String(s.to_lowercase().into()));
                            }
                            "startsWith" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.starts_with(search)));
                            }
                            "endsWith" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.ends_with(search)));
                            }
                            "replace" => {
                                #[cfg(feature = "regex")]
                                if let (Some(search), Some(replace)) = (arg_vals.first(), arg_vals.get(1)) {
                                    let is_fn = matches!(replace, Value::Function { .. } | Value::Native(_));
                                    if matches!(search, Value::RegExp(_)) && is_fn {
                                        let re = match search {
                                            Value::RegExp(r) => r.clone(),
                                            _ => unreachable!(),
                                        };
                                        let re_guard = re.borrow();
                                        let replace_fn = replace.clone();
                                        let input_str = s.as_ref();
                                        let mut invoke = |args: &[Value]| {
                                            self.call_func(&replace_fn, args)
                                                .map(|v| v.to_string())
                                                .map_err(|e: EvalError| e.to_string())
                                        };
                                        match crate::regex::string_replace_regex_with_fn(
                                            input_str,
                                            &re_guard,
                                            &mut invoke,
                                        ) {
                                            Ok(v) => return Ok(v),
                                            Err(_) => return Ok(Value::String(Arc::clone(s))),
                                        }
                                    }
                                    return Ok(crate::regex::string_replace(s.as_ref(), search, replace));
                                }
                                #[cfg(not(feature = "regex"))]
                                {
                                    let search = match arg_vals.first() {
                                        Some(Value::String(ss)) => ss.to_string(),
                                        _ => return Ok(obj.clone()),
                                    };
                                    let replacement = match arg_vals.get(1) {
                                        Some(Value::String(ss)) => ss.to_string(),
                                        _ => String::new(),
                                    };
                                    return Ok(Value::String(s.replacen(&search, &replacement, 1).into()));
                                }
                                #[cfg(feature = "regex")]
                                return Ok(obj.clone());
                            }
                            "replaceAll" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.to_string(),
                                    _ => return Ok(obj.clone()),
                                };
                                let replacement = match arg_vals.get(1) {
                                    Some(Value::String(ss)) => ss.to_string(),
                                    _ => String::new(),
                                };
                                return Ok(Value::String(s.replace(&search, &replacement).into()));
                            }
                            "charAt" => {
                                // Cursor cache instead of collecting a fresh Vec<char> each call (#203).
                                let idx = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 0,
                                };
                                return Ok(nth_char_cached(s, idx)
                                    .map(|c| Value::String(c.to_string().into()))
                                    .unwrap_or(Value::String("".into())));
                            }
                            "at" => {
                                // Negative index from end; out of range → null (#247).
                                let i = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as i64,
                                    _ => 0,
                                };
                                let idx = if i < 0 { s.chars().count() as i64 + i } else { i };
                                if idx >= 0 {
                                    if let Some(c) = nth_char_cached(s, idx as usize) {
                                        return Ok(Value::String(c.to_string().into()));
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "charCodeAt" => {
                                let idx = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 0,
                                };
                                return Ok(nth_char_cached(s, idx)
                                    .map(|c| Value::Number(c as u32 as f64))
                                    .unwrap_or(Value::Number(f64::NAN)));
                            }
                            "repeat" => {
                                let count = match arg_vals.first() {
                                    Some(Value::Number(n)) if *n >= 0.0 => *n as usize,
                                    _ => 0,
                                };
                                return Ok(Value::String(s.repeat(count).into()));
                            }
                            "padStart" => {
                                let target_len = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => return Ok(obj.clone()),
                                };
                                let pad = match arg_vals.get(1) {
                                    Some(Value::String(p)) => p.to_string(),
                                    _ => " ".to_string(),
                                };
                                let char_count = s.chars().count();
                                if char_count >= target_len || pad.is_empty() {
                                    return Ok(obj.clone());
                                }
                                let needed = target_len - char_count;
                                let padding: String = pad.chars().cycle().take(needed).collect();
                                return Ok(Value::String(format!("{}{}", padding, s).into()));
                            }
                            "padEnd" => {
                                let target_len = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => return Ok(obj.clone()),
                                };
                                let pad = match arg_vals.get(1) {
                                    Some(Value::String(p)) => p.to_string(),
                                    _ => " ".to_string(),
                                };
                                let char_count = s.chars().count();
                                if char_count >= target_len || pad.is_empty() {
                                    return Ok(obj.clone());
                                }
                                let needed = target_len - char_count;
                                let padding: String = pad.chars().cycle().take(needed).collect();
                                return Ok(Value::String(format!("{}{}", s, padding).into()));
                            }
                            #[cfg(feature = "regex")]
                            "match" => {
                                if let Some(regexp) = arg_vals.first() {
                                    return Ok(crate::regex::string_match(s, regexp));
                                }
                                return Ok(Value::Null);
                            }
                            #[cfg(feature = "regex")]
                            "search" => {
                                if let Some(regexp) = arg_vals.first() {
                                    return Ok(crate::regex::string_search(s, regexp));
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            _ => {}
                        }
                    }

                    // Number methods
                    if let Value::Number(n) = &obj {
                        if method_name.as_ref() == "toFixed" {
                            let digits = arg_vals
                                .first()
                                .and_then(|v| match v {
                                    Value::Number(d) => Some(*d as i32),
                                    _ => None,
                                })
                                .unwrap_or(0)
                                .clamp(0, 20); // ECMA-262: 0–20
                            // Shared half-away-from-zero rounding so interp matches vm/native/node (#247).
                            let formatted = tishlang_builtins::number::to_fixed_str(*n, digits as usize);
                            return Ok(Value::String(formatted.into()));
                        }
                        if method_name.as_ref() == "toString" {
                            // Shares the VM/native formatting via the backend-agnostic helper
                            // (issue #59). Radix defaults to 10; 2–36 supported, else RangeError.
                            let radix = arg_vals
                                .first()
                                .and_then(|v| match v {
                                    Value::Number(d) => Some(*d as i64),
                                    _ => None,
                                })
                                .unwrap_or(10);
                            return match tishlang_builtins::number::number_to_string_radix(*n, radix)
                            {
                                Some(s) => Ok(Value::String(s.into())),
                                None => Err(EvalError::Error(
                                    "toString() radix must be between 2 and 36".to_string(),
                                )),
                            };
                        }
                    }

                    // RegExp methods
                    #[cfg(feature = "regex")]
                    if let Value::RegExp(re) = &obj {
                        match method_name.as_ref() {
                            "test" => {
                                let input = arg_vals.first()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let result = re.borrow_mut().test(&input);
                                return Ok(Value::Bool(result));
                            }
                            "exec" => {
                                let input = arg_vals.first()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let result = crate::regex::regexp_exec(&mut re.borrow_mut(), &input);
                                return Ok(result);
                            }
                            _ => {}
                        }
                    }
                    
                    // Fall through to normal function call. `get_prop` only implements `length` on
                    // strings, so method calls would otherwise become `call_func(Null)` → Not a function.
                    if let Value::String(s) = &obj {
                        if method_name.as_ref() == "lastIndexOf" {
                            return Ok(Self::string_last_index_of_eval(&arg_vals, s));
                        }
                    }
                    let f = self.get_prop(&obj, method_name).map_err(EvalError::Error)?;
                    return self.call_func(&f, &arg_vals);
                }
                
                let f = self.eval_expr(callee)?;
                let arg_vals = self.eval_call_args(args)?;
                self.call_func(&f, &arg_vals)
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                let obj = self.eval_expr(object)?;
                if *optional && matches!(obj, Value::Null) {
                    return Ok(Value::Null);
                }
                let key = match prop {
                    MemberProp::Name { name, .. } => Arc::clone(name),
                    MemberProp::Expr(e) => {
                        let v = self.eval_expr(e)?;
                        match v {
                            Value::String(s) => s,
                            _ => return Err(EvalError::Error("Property key must be string".to_string())),
                        }
                    }
                };
                match self.get_prop(&obj, &key) {
                    Ok(v) => Ok(v),
                    Err(_) if *optional => Ok(Value::Null),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::Index {
                object,
                index,
                optional,
                ..
            } => {
                let obj = self.eval_expr(object)?;
                if *optional && matches!(obj, Value::Null) {
                    return Ok(Value::Null);
                }
                let idx = self.eval_expr(index)?;
                self.get_index(&obj, &idx).map_err(EvalError::Error)
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                if self.eval_expr(cond)?.is_truthy() {
                    self.eval_expr(then_branch)
                } else {
                    self.eval_expr(else_branch)
                }
            }
            Expr::NullishCoalesce { left, right, .. } => {
                let l = self.eval_expr(left)?;
                if matches!(l, Value::Null) {
                    self.eval_expr(right)
                } else {
                    Ok(l)
                }
            }
            Expr::Array { elements, .. } => {
                let mut vals = Vec::with_capacity(elements.len());
                for elem in elements {
                    match elem {
                        tishlang_ast::ArrayElement::Expr(e) => {
                            vals.push(self.eval_expr(e)?);
                        }
                        tishlang_ast::ArrayElement::Spread(e) => {
                            let spread_val = self.eval_expr(e)?;
                            if let Value::Array(arr) = &spread_val {
                                vals.extend(arr.borrow().iter().cloned());
                            } else if let Some(items) = self.drain_eval_iterator(&spread_val) {
                                // Spread a Map/Set iterator (`[...m.values()]`).
                                vals.extend(items);
                            }
                        }
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(vals))))
            }
            Expr::Object { props, .. } => {
                let mut data = EvalObjectData::default();
                for prop in props {
                    match prop {
                        tishlang_ast::ObjectProp::KeyValue(k, v, _) => {
                            data
                                .strings
                                .insert(Arc::clone(k), self.eval_expr(v)?);
                        }
                        tishlang_ast::ObjectProp::Spread(e) => {
                            let spread_val = self.eval_expr(e)?;
                            if let Value::Object(obj) = spread_val {
                                let b = obj.borrow();
                                for (k, v) in b.strings.iter() {
                                    data.strings.insert(Arc::clone(k), v.clone());
                                }
                                if let Some(ref sm) = b.symbols {
                                    if data.symbols.is_none() {
                                        data.symbols = Some(AHashMap::default());
                                    }
                                    let dm = data.symbols.as_mut().unwrap();
                                    for (id, v) in sm.iter() {
                                        dm.insert(*id, v.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Value::Object(Rc::new(RefCell::new(data))))
            }
            Expr::Assign { name, value, .. } => {
                let v = self.eval_expr(value)?;
                // A plain assignment overwrites the variable, so drop any builder buffering it (the
                // buffered value is about to be replaced). `value` may itself have read+flushed it.
                self.discard_pending_for(name.as_ref());
                match self.scope.borrow_mut().assign(name.as_ref(), v.clone()) {
                    Ok(true) => Ok(v),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::Await { operand, .. } => self.eval_await(operand),
            Expr::New { callee, args, .. } => {
                let c = self.eval_expr(callee)?;
                let arg_vals = self.eval_call_args(args)?;
                self.construct_value(&c, &arg_vals)
            }
            Expr::JsxElement { .. } | Expr::JsxFragment { .. } => Err(EvalError::Error(
                "JSX is not supported in the interpreter. Use 'tish build --target js' to compile to JavaScript.".to_string(),
            )),
            Expr::NativeModuleLoad { spec, export_name, .. } => {
                self.load_builtin_export(spec.as_ref(), export_name.as_ref())
            }
            Expr::TypeOf { operand, .. } => {
                let v = self.eval_expr(operand)?;
                Ok(Value::String(match &v {
                    Value::Number(_) => "number".into(),
                    Value::String(_) => "string".into(),
                    Value::Bool(_) => "boolean".into(),
                    Value::Null => "null".into(),
                    Value::Array(_) => "object".into(),
                    Value::Object(_) => "object".into(),
                    Value::Symbol(_) => "symbol".into(),
                    Value::Function { .. } | Value::Native(_) => "function".into(),
                    Value::CoreFn(_) => "function".into(),
                    #[cfg(feature = "http")]
                    Value::CorePromise(_) => "object".into(),
                    #[cfg(feature = "http")]
                    Value::Serve
                    | Value::PromiseResolver(_)
                    | Value::PromiseConstructor
                    | Value::BoundPromiseMethod(_, _) => "function".into(),
                    #[cfg(feature = "timers")]
                    Value::TimerBuiltin(_) => "function".into(),
                    #[cfg(feature = "http")]
                    Value::Promise(_) => "object".into(),
                    #[cfg(feature = "regex")]
                    Value::RegExp(_) => "object".into(),
                    Value::Opaque(_) => "object".into(),
                    Value::OpaqueMethod(_, _) => "function".into(),
                }))
            }
            // `delete obj.prop` / `delete obj[key]` (issue #40): remove the property and
            // evaluate to `true`. Objects drop the key; arrays clear a numeric index to a
            // null hole (length preserved). Deleting a non-reference is a no-op (still `true`).
            Expr::Delete { target, .. } => {
                // Resolve the target to (object value, key value); then remove the key.
                let resolved = match target.as_ref() {
                    Expr::Member { object, prop: MemberProp::Name { name, .. }, .. } => {
                        Some((self.eval_expr(object)?, Value::String(name.as_ref().into())))
                    }
                    Expr::Member { object, prop: MemberProp::Expr(key), .. } => {
                        Some((self.eval_expr(object)?, self.eval_expr(key)?))
                    }
                    Expr::Index { object, index, .. } => {
                        Some((self.eval_expr(object)?, self.eval_expr(index)?))
                    }
                    _ => None,
                };
                if let Some((obj, key)) = resolved {
                    match &obj {
                        Value::Object(map) => {
                            let key_s = match &key {
                                Value::String(s) => s.to_string(),
                                Value::Number(n) => n.to_string(),
                                other => other.to_string(),
                            };
                            // shift_remove preserves the insertion order of the remaining keys
                            // (JS delete semantics); plain remove() is deprecated on IndexMap.
                            map.borrow_mut().strings.shift_remove(key_s.as_str());
                        }
                        Value::Array(arr) => {
                            if let Value::Number(n) = &key {
                                let n = *n;
                                if n >= 0.0 && n.fract() == 0.0 {
                                    let i = n as usize;
                                    let mut a = arr.borrow_mut();
                                    if i < a.len() {
                                        a[i] = Value::Null;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Value::Bool(true))
            }
            Expr::PostfixInc { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply ++ to {:?}", v))),
                };
                match self.scope.borrow_mut().assign(name.as_ref(), Value::Number(n + 1.0)) {
                    Ok(true) => Ok(Value::Number(n)),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::PostfixDec { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply -- to {:?}", v))),
                };
                match self.scope.borrow_mut().assign(name.as_ref(), Value::Number(n - 1.0)) {
                    Ok(true) => Ok(Value::Number(n)),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::PrefixInc { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply ++ to {:?}", v))),
                };
                let new_val = Value::Number(n + 1.0);
                match self.scope.borrow_mut().assign(name.as_ref(), new_val.clone()) {
                    Ok(true) => Ok(new_val),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::PrefixDec { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply -- to {:?}", v))),
                };
                let new_val = Value::Number(n - 1.0);
                match self.scope.borrow_mut().assign(name.as_ref(), new_val.clone()) {
                    Ok(true) => Ok(new_val),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::CompoundAssign { name, op, value, .. } => {
                // Expression-position `+=` (result is used): flush any builder so `current` is the
                // full string, then take the generic path. The O(1) builder fast path is reserved
                // for statement position (see `eval_expr_discard`).
                self.flush_pending_for(name.as_ref());
                let current = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let rhs = self.eval_expr(value)?;
                let bin_op = match op {
                    CompoundOp::Add => BinOp::Add,
                    CompoundOp::Sub => BinOp::Sub,
                    CompoundOp::Mul => BinOp::Mul,
                    CompoundOp::Div => BinOp::Div,
                    CompoundOp::Mod => BinOp::Mod,
                };
                let result = self.eval_binop(&current, bin_op, &rhs).map_err(EvalError::Error)?;
                match self.scope.borrow_mut().assign(name.as_ref(), result.clone()) {
                    Ok(true) => Ok(result),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::LogicalAssign { name, op, value, .. } => {
                self.flush_pending_for(name.as_ref());
                let current = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let result = match op {
                    LogicalAssignOp::AndAnd => {
                        if current.is_truthy() {
                            let rhs = self.eval_expr(value)?;
                            let _ = self.scope.borrow_mut().assign(name.as_ref(), rhs.clone());
                            rhs
                        } else {
                            current.clone()
                        }
                    }
                    LogicalAssignOp::OrOr => {
                        if !current.is_truthy() {
                            let rhs = self.eval_expr(value)?;
                            let _ = self.scope.borrow_mut().assign(name.as_ref(), rhs.clone());
                            rhs
                        } else {
                            current.clone()
                        }
                    }
                    LogicalAssignOp::Nullish => {
                        if matches!(current, Value::Null) {
                            let rhs = self.eval_expr(value)?;
                            let _ = self.scope.borrow_mut().assign(name.as_ref(), rhs.clone());
                            rhs
                        } else {
                            current.clone()
                        }
                    }
                };
                Ok(result)
            }
            Expr::MemberAssign { object, prop, value, .. } => {
                let obj_val = self.eval_expr(object)?;
                let val = self.eval_expr(value)?;
                match obj_val {
                    Value::Object(map) => {
                        map.borrow_mut()
                            .strings
                            .insert(Arc::clone(prop), val.clone());
                        Ok(val)
                    }
                    // `arr.length = k` truncates / grows the array (holes read back as Null),
                    // matching JS and the bytecode VM (issue #62).
                    Value::Array(arr) if prop.as_ref() == "length" => {
                        let n = match &val {
                            Value::Number(n) => *n,
                            _ => f64::NAN,
                        };
                        if n.is_nan() || n < 0.0 || n.fract() != 0.0 || n > 4_294_967_295.0 {
                            return Err(EvalError::Error("Invalid array length".to_string()));
                        }
                        arr.borrow_mut().resize(n as usize, Value::Null);
                        Ok(val)
                    }
                    _ => Err(EvalError::Error(format!(
                        "Cannot assign property '{}' on non-object: {:?}",
                        prop, obj_val
                    ))),
                }
            }
            Expr::IndexAssign { object, index, value, .. } => {
                let obj_val = self.eval_expr(object)?;
                let idx_val = self.eval_expr(index)?;
                let val = self.eval_expr(value)?;
                match obj_val {
                    Value::Array(arr) => {
                        let idx = match &idx_val {
                            Value::Number(n) => *n as usize,
                            _ => return Err(EvalError::Error(format!(
                                "Array index must be a number, got {:?}",
                                idx_val
                            ))),
                        };
                        let mut arr_mut = arr.borrow_mut();
                        // Extend array if necessary (JS behavior)
                        while arr_mut.len() <= idx {
                            arr_mut.push(Value::Null);
                        }
                        arr_mut[idx] = val.clone();
                        Ok(val)
                    }
                    Value::Object(_) => {
                        eval_object_set(&obj_val, &idx_val, val.clone())
                            .map_err(EvalError::Error)?;
                        Ok(val)
                    }
                    _ => Err(EvalError::Error(format!(
                        "Cannot assign index on non-array/object: {:?}",
                        obj_val
                    ))),
                }
            }
            Expr::ArrowFunction { params, body, .. } => {
                use tishlang_ast::ArrowBody;
                let formals: Arc<[FunParam]> = Arc::from(params.clone());
                let body_stmt = match body {
                    ArrowBody::Expr(expr) => {
                        // Expression body: wrap in implicit return
                        Statement::Return {
                            value: Some(expr.as_ref().clone()),
                            span: Span { start: (0, 0), end: (0, 0) },
                        }
                    }
                    ArrowBody::Block(stmt) => stmt.as_ref().clone(),
                };
                Ok(Value::Function {
                    formals,
                    rest_param: None,
                    body: Arc::new(body_stmt),
                    env: Rc::clone(&self.scope),
                })
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                // Build the string by interleaving quasis and evaluated expressions
                let mut result = String::new();
                for (i, quasi) in quasis.iter().enumerate() {
                    result.push_str(quasi);
                    if i < exprs.len() {
                        let val = self.eval_expr(&exprs[i])?;
                        result.push_str(&val.to_js_string());
                    }
                }
                Ok(Value::String(result.into()))
            }
        }
    }

    fn eval_binop(&self, l: &Value, op: BinOp, r: &Value) -> Result<Value, String> {
        match op {
            BinOp::Add => match (l, r) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                (Value::String(a), Value::String(b)) => {
                    let mut s = String::with_capacity(a.len() + b.len());
                    s.push_str(a);
                    s.push_str(b);
                    Ok(Value::String(s.into()))
                }
                (Value::String(a), b) => {
                    let b_str = b.to_js_string();
                    let mut s = String::with_capacity(a.len() + b_str.len());
                    s.push_str(a);
                    s.push_str(&b_str);
                    Ok(Value::String(s.into()))
                }
                (a, Value::String(b)) => {
                    let a_str = a.to_js_string();
                    let mut s = String::with_capacity(a_str.len() + b.len());
                    s.push_str(&a_str);
                    s.push_str(b);
                    Ok(Value::String(s.into()))
                }
                // Neither operand is a string: numeric add, coercing non-numbers
                // (Null/Bool/Object/…) to NaN exactly like the VM's
                // `as_number().unwrap_or(NaN)` (vm.rs eval_binop). e.g. an out-of-bounds
                // array read is `Null` (JS `undefined`), so `15 + arr[oob]` → NaN, not an error.
                _ => Ok(Value::Number(
                    l.as_number().unwrap_or(f64::NAN) + r.as_number().unwrap_or(f64::NAN),
                )),
            },
            BinOp::Sub => self.binop_number(l, r, |a, b| Value::Number(a - b)),
            BinOp::Mul => self.binop_number(l, r, |a, b| Value::Number(a * b)),
            BinOp::Div => self.binop_number(l, r, |a, b| Value::Number(a / b)),
            BinOp::Mod => self.binop_number(l, r, |a, b| Value::Number(a % b)),
            BinOp::Pow => self.binop_number(l, r, |a, b| Value::Number(a.powf(b))),
            BinOp::StrictEq => Ok(Value::Bool(l.strict_eq(r))),
            BinOp::StrictNe => Ok(Value::Bool(!l.strict_eq(r))),
            // Relational ops compare strings lexicographically when BOTH operands
            // are strings (JS semantics); otherwise coerce to numbers via binop_number.
            BinOp::Lt => self.binop_relational(l, r, |o| o.is_lt()),
            BinOp::Le => self.binop_relational(l, r, |o| o.is_le()),
            BinOp::Gt => self.binop_relational(l, r, |o| o.is_gt()),
            BinOp::Ge => self.binop_relational(l, r, |o| o.is_ge()),
            BinOp::And => Ok(Value::Bool(l.is_truthy() && r.is_truthy())),
            BinOp::Or => Ok(Value::Bool(l.is_truthy() || r.is_truthy())),
            BinOp::BitAnd => self.binop_int32(l, r, |a, b| Value::Number((a & b) as f64)),
            BinOp::BitOr => self.binop_int32(l, r, |a, b| Value::Number((a | b) as f64)),
            BinOp::BitXor => self.binop_int32(l, r, |a, b| Value::Number((a ^ b) as f64)),
            // JS shifts mask the count to the low 5 bits; `wrapping_sh*` does exactly
            // that and never panics (plain `<<`/`>>` panic in debug for count >= 32).
            BinOp::Shl => {
                self.binop_int32(l, r, |a, b| Value::Number(a.wrapping_shl(b as u32) as f64))
            }
            BinOp::Shr => {
                self.binop_int32(l, r, |a, b| Value::Number(a.wrapping_shr(b as u32) as f64))
            }
            // `>>>` — unsigned (logical) right shift: ToUint32(a) >>> (b & 31).
            BinOp::UShr => self.binop_int32(l, r, |a, b| {
                Value::Number((a as u32).wrapping_shr(b as u32) as f64)
            }),
            BinOp::In => {
                let ok = match r {
                    Value::Object(_) => eval_object_has(r, l),
                    Value::Array(arr) => {
                        let key: Arc<str> = match l {
                            Value::String(s) => Arc::clone(s),
                            Value::Number(n) => n.to_string().into(),
                            _ => {
                                return Err(format!(
                                    "'in' requires string or number key on array, got {:?}",
                                    l
                                ))
                            }
                        };
                        key.as_ref() == "length"
                            || key
                                .parse::<usize>()
                                .ok()
                                .map(|i| i < arr.borrow().len())
                                .unwrap_or(false)
                    }
                    _ => return Err(format!("'in' requires object or array, got {:?}", r)),
                };
                Ok(Value::Bool(ok))
            }
            // Loose ==/!= : match the VM (vm.rs maps Eq/Ne to strict_eq) so interp == vm ==
            // compiled. Previously the interpreter alone errored on `==`.
            BinOp::Eq => Ok(Value::Bool(l.strict_eq(r))),
            BinOp::Ne => Ok(Value::Bool(!l.strict_eq(r))),
        }
    }

    /// Check if a function value is the common numeric sort comparator pattern.
    /// descending = false: checks for `(a, b) => a - b`
    /// descending = true: checks for `(a, b) => b - a`
    fn is_numeric_sort_comparator(f: &Value, descending: bool) -> bool {
        if let Value::Function {
            formals,
            body,
            rest_param,
            ..
        } = f
        {
            // Must have exactly 2 simple params, no defaults, no rest
            if formals.len() != 2 || rest_param.is_some() {
                return false;
            }
            let (param_a, param_b) = match (&formals[0], &formals[1]) {
                (FunParam::Simple(a), FunParam::Simple(b))
                    if a.default.is_none() && b.default.is_none() =>
                {
                    (&a.name, &b.name)
                }
                _ => return false,
            };

            // Body must be a return of a - b (or b - a for descending)

            // Check for both Statement::Return and Statement::ExprStmt (arrow implicit return)
            let expr = match body.as_ref() {
                Statement::Return { value: Some(e), .. } => e,
                Statement::ExprStmt { expr: e, .. } => e,
                _ => return false,
            };

            // Check for binary subtraction
            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = expr
            {
                // Check left is Ident(a) and right is Ident(b)
                let (expected_left, expected_right) = if descending {
                    (param_b, param_a) // b - a
                } else {
                    (param_a, param_b) // a - b
                };

                if let (
                    Expr::Ident {
                        name: left_name, ..
                    },
                    Expr::Ident {
                        name: right_name, ..
                    },
                ) = (left.as_ref(), right.as_ref())
                {
                    return left_name == expected_left && right_name == expected_right;
                }
            }
        }
        false
    }

    /// JS ToInt32 coercion. Non-numbers coerce to NaN → 0. Going through `i64`
    /// (not a direct `as i32`) gives modulo-2³² truncation instead of a saturating
    /// cast, so out-of-i32-range values (e.g. a `0..2³²` hash) wrap exactly like JS:
    /// `4294967295 | 0 === -1`, not the saturated `i32::MAX`. Realistic values are
    /// `< 2⁵³` so they fit `i64` exactly; the two casts stay cheap.
    fn to_int32(v: &Value) -> i32 {
        // NaN / ±Infinity → 0 (the `is_finite` guard): `f64 as i64` *saturates* (`+∞ → i64::MAX
        // → -1`), which is not the JS ToInt32 result. Finite values use the cheap modulo cast.
        let x = v.as_number().unwrap_or(f64::NAN);
        if x.is_finite() {
            x as i64 as i32
        } else {
            0
        }
    }

    fn binop_int32<F>(&self, l: &Value, r: &Value, f: F) -> Result<Value, String>
    where
        F: FnOnce(i32, i32) -> Value,
    {
        Ok(f(Self::to_int32(l), Self::to_int32(r)))
    }

    /// Numeric binop, coercing each operand to a number the way the VM does
    /// (`as_number().unwrap_or(NaN)`): non-numbers (Null/Bool/Object/…) become NaN rather
    /// than erroring. Keeps the interpreter in parity with the VM and Node on out-of-bounds
    /// reads and other `undefined`-like operands.
    fn binop_number<F>(&self, l: &Value, r: &Value, f: F) -> Result<Value, String>
    where
        F: FnOnce(f64, f64) -> Value,
    {
        let a = l.as_number().unwrap_or(f64::NAN);
        let b = r.as_number().unwrap_or(f64::NAN);
        Ok(f(a, b))
    }

    /// Relational comparison (`<` `<=` `>` `>=`). When both operands are strings,
    /// compare lexicographically; otherwise coerce to numbers. `pred` maps the
    /// resulting `Ordering` to a bool. A NaN-involved numeric comparison yields no
    /// ordering and is always `false`, matching JS (`NaN < 5` → false).
    fn binop_relational<F>(&self, l: &Value, r: &Value, pred: F) -> Result<Value, String>
    where
        F: FnOnce(std::cmp::Ordering) -> bool,
    {
        let ord = match (l, r) {
            (Value::String(a), Value::String(b)) => Some(a.as_ref().cmp(b.as_ref())),
            _ => {
                let a = l.as_number().unwrap_or(f64::NAN);
                let b = r.as_number().unwrap_or(f64::NAN);
                a.partial_cmp(&b)
            }
        };
        Ok(Value::Bool(ord.map(pred).unwrap_or(false)))
    }

    fn eval_unary(&self, op: UnaryOp, v: &Value) -> Result<Value, String> {
        match op {
            UnaryOp::Not => Ok(Value::Bool(!v.is_truthy())),
            UnaryOp::Neg => match v {
                Value::Number(n) => Ok(Value::Number(-n)),
                _ => Err(format!("Cannot negate {:?}", v)),
            },
            UnaryOp::Pos => match v {
                Value::Number(n) => Ok(Value::Number(*n)),
                _ => Err(format!("Cannot apply unary + to {:?}", v)),
            },
            UnaryOp::BitNot => {
                let n = Self::to_int32(v);
                Ok(Value::Number((!n) as f64))
            }
            UnaryOp::Void => Ok(Value::Null),
        }
    }

    /// Optimized callback invocation for array methods.
    /// Creates a reusable scope that can be updated for each iteration.
    fn create_callback_scope(
        &self,
        f: &Value,
    ) -> Option<(Rc<RefCell<Scope>>, Arc<[Arc<str>]>, Arc<Statement>)> {
        if let Value::Function {
            formals,
            body,
            rest_param,
            ..
        } = f
        {
            if rest_param.is_some() {
                return None;
            }
            for fp in formals.iter() {
                match fp {
                    FunParam::Simple(tp) if tp.default.is_none() => {}
                    _ => return None,
                }
            }
            let scope = Scope::child(Rc::clone(&self.scope));
            {
                let mut s = scope.borrow_mut();
                for fp in formals.iter() {
                    for n in fp.bound_names() {
                        s.set(n, Value::Null, true);
                    }
                }
            }
            let flat_names: Arc<[Arc<str>]> = Arc::from(
                formals
                    .iter()
                    .flat_map(|fp| fp.bound_names())
                    .collect::<Vec<_>>(),
            );
            return Some((scope, flat_names, Arc::clone(body)));
        }
        None
    }

    /// Fast callback invocation that reuses an existing scope.
    fn call_with_scope(
        &self,
        scope: &Rc<RefCell<Scope>>,
        params: &[Arc<str>],
        body: &Statement,
        args: &[Value],
    ) -> Result<Value, EvalError> {
        {
            let mut s = scope.borrow_mut();
            for (i, p) in params.iter().enumerate() {
                let val = args.get(i).cloned().unwrap_or(Value::Null);
                // Direct assignment - we know these vars exist and are mutable
                if let Some(existing) = s.vars.get_mut(p.as_ref()) {
                    *existing = val;
                }
            }
        }
        let mut eval = Evaluator {
            scope: Rc::clone(scope),
            module_cache: Rc::clone(&self.module_cache),
            current_dir: RefCell::new(self.current_dir.borrow().clone()),
            virtual_builtins: Rc::clone(&self.virtual_builtins),
            string_builder: Rc::clone(&self.string_builder),
            call_depth: Rc::clone(&self.call_depth),
            max_call_depth: self.max_call_depth,
        };
        match eval.eval_statement(body) {
            Ok(v) => Ok(v),
            Err(EvalError::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Try to evaluate a simple callback expression directly without creating a scope.
    /// Returns Some(result) for simple patterns like `x => x * 2` or `x => x > 5`.
    fn eval_simple_callback(&self, f: &Value, args: &[Value]) -> Option<Result<Value, EvalError>> {
        if let Value::Function {
            formals,
            body,
            rest_param,
            ..
        } = f
        {
            if formals.len() != 1 || rest_param.is_some() {
                return None;
            }
            let param_name = match &formals[0] {
                FunParam::Simple(tp) if tp.default.is_none() => &tp.name,
                _ => return None,
            };
            let arg = args.first().cloned().unwrap_or(Value::Null);

            // Get the expression from the body
            let expr = match body.as_ref() {
                Statement::Return { value: Some(e), .. } => e,
                Statement::ExprStmt { expr: e, .. } => e,
                _ => return None,
            };

            // Fast path for common patterns
            match expr {
                // x * constant or x + constant, etc.
                Expr::Binary {
                    left, op, right, ..
                } => {
                    let left_val = self.eval_simple_operand(left, param_name, &arg)?;
                    let right_val = self.eval_simple_operand(right, param_name, &arg)?;
                    Some(
                        self.eval_binop(&left_val, *op, &right_val)
                            .map_err(EvalError::Error),
                    )
                }
                // Just return the parameter
                Expr::Ident { name, .. } if name == param_name => Some(Ok(arg)),
                // Property access: x.prop
                Expr::Member {
                    object,
                    prop,
                    optional,
                    ..
                } => {
                    if let Expr::Ident { name, .. } = object.as_ref() {
                        if name == param_name {
                            return self.eval_simple_member(&arg, prop, *optional);
                        }
                    }
                    None
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Evaluate a simple operand (identifier or literal).
    fn eval_simple_operand(
        &self,
        expr: &Expr,
        param_name: &Arc<str>,
        param_val: &Value,
    ) -> Option<Value> {
        match expr {
            Expr::Ident { name, .. } if name == param_name => Some(param_val.clone()),
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => Some(Value::Number(*n)),
                Literal::String(s) => Some(Value::String(Arc::clone(s))),
                Literal::Bool(b) => Some(Value::Bool(*b)),
                Literal::Null => Some(Value::Null),
            },
            _ => None,
        }
    }

    /// Evaluate simple member access.
    fn eval_simple_member(
        &self,
        obj: &Value,
        property: &MemberProp,
        _optional: bool,
    ) -> Option<Result<Value, EvalError>> {
        match property {
            MemberProp::Name { name, .. } => match obj {
                Value::Object(o) => {
                    let result = o
                        .borrow()
                        .strings
                        .get(name.as_ref())
                        .cloned()
                        .unwrap_or(Value::Null);
                    Some(Ok(result))
                }
                Value::Array(arr) if name.as_ref() == "length" => {
                    Some(Ok(Value::Number(arr.borrow().len() as f64)))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Host `new`: `__construct` on objects; otherwise same callables as `call_func`, else null.
    fn construct_value(&self, callee: &Value, args: &[Value]) -> Result<Value, EvalError> {
        if let Value::Object(o) = callee {
            if let Some(ctor) = o
                .borrow()
                .strings
                .get("__construct")
                .cloned()
            {
                return self.call_func(&ctor, args);
            }
        }
        match callee {
            Value::Native(_) | Value::Function { .. } | Value::CoreFn(_) => {
                self.call_func(callee, args)
            }
            #[cfg(feature = "http")]
            Value::PromiseConstructor | Value::Serve | Value::BoundPromiseMethod(_, _) => {
                self.call_func(callee, args)
            }
            #[cfg(feature = "timers")]
            Value::TimerBuiltin(_) => self.call_func(callee, args),
            Value::OpaqueMethod(_, _) => self.call_func(callee, args),
            _ => Ok(Value::Null),
        }
    }

    fn call_func(&self, f: &Value, args: &[Value]) -> Result<Value, EvalError> {
        match f {
            Value::Object(o) => {
                if let Some(call) = o.borrow().strings.get("__call").cloned() {
                    return self.call_func(&call, args);
                }
                Err(EvalError::Error("Not a function".to_string()))
            }
            Value::Native(native_fn) => native_fn(args).map_err(EvalError::Error),
            #[cfg(feature = "http")]
            Value::PromiseResolver(r) => {
                let value = args.first().cloned().unwrap_or(Value::Null);
                let (val, is_fulfilled, reactions) =
                    crate::promise::settle_promise(r, value, r.is_resolve)
                        .map_err(EvalError::Error)?;
                for reaction in reactions {
                    match reaction {
                        crate::promise::Reaction::Then(
                            on_fulfilled,
                            on_rejected,
                            ref resolve,
                            ref reject,
                        ) => {
                            let handler_result = if is_fulfilled {
                                if let Some(ref h) = on_fulfilled {
                                    self.call_func(h, &[val.clone()])
                                } else {
                                    Ok(val.clone())
                                }
                            } else {
                                if let Some(ref h) = on_rejected {
                                    self.call_func(h, &[val.clone()])
                                } else {
                                    Err(EvalError::Throw(val.clone()))
                                }
                            };
                            match handler_result {
                                Ok(v) => {
                                    crate::promise::settle_promise(resolve, v, true)
                                        .map_err(EvalError::Error)?;
                                }
                                Err(EvalError::Throw(v)) => {
                                    crate::promise::settle_promise(reject, v, false)
                                        .map_err(EvalError::Error)?;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        crate::promise::Reaction::Finally(on_finally, ref resolve, ref reject) => {
                            let _ = self.call_func(&on_finally, &[]);
                            if is_fulfilled {
                                crate::promise::settle_promise(resolve, val.clone(), true)
                                    .map_err(EvalError::Error)?;
                            } else {
                                crate::promise::settle_promise(reject, val.clone(), false)
                                    .map_err(EvalError::Error)?;
                            }
                        }
                    }
                }
                Ok(Value::Null)
            }
            #[cfg(feature = "http")]
            Value::PromiseConstructor => {
                let executor = args.first().ok_or_else(|| {
                    EvalError::Error("Promise requires an executor function".to_string())
                })?;
                let (promise, resolve, reject) = crate::promise::create_promise();
                self.call_func(executor, &[resolve, reject])?;
                Ok(promise)
            }
            #[cfg(feature = "http")]
            Value::Serve => self.run_http_server(args),
            Value::CoreFn(f) => {
                let ca: Result<Vec<tishlang_core::Value>, String> = args
                    .iter()
                    .map(crate::value_convert::eval_to_core)
                    .collect();
                let ca = ca.map_err(EvalError::Error)?;
                Ok(crate::value_convert::core_to_eval(f.call(&ca)))
            }
            #[cfg(feature = "regex")]
            Value::RegExp(_) => Err(EvalError::Error("RegExp is not callable".to_string())),
            #[cfg(feature = "http")]
            Value::BoundPromiseMethod(promise_ref, method) => {
                self.run_promise_method(promise_ref, method.as_ref(), args)
            }
            #[cfg(feature = "timers")]
            Value::TimerBuiltin(name) => self.run_timer_builtin(name.as_ref(), args),
            Value::OpaqueMethod(opaque, method_name) => {
                let method = opaque.get_method(method_name.as_ref()).ok_or_else(|| {
                    EvalError::Error(format!(
                        "Method {} not found on {}",
                        method_name,
                        opaque.type_name()
                    ))
                })?;
                let core_args: Result<Vec<tishlang_core::Value>, String> = args
                    .iter()
                    .map(crate::value_convert::eval_to_core)
                    .collect();
                let core_args = core_args.map_err(EvalError::Error)?;
                let result = method.call(&core_args);
                Ok(crate::value_convert::core_to_eval(result))
            }
            Value::Function {
                formals,
                rest_param,
                body,
                env,
            } => {
                // A real closure: the call frame's parent is the function's DEFINING scope (env),
                // not the call site — so free variables resolve lexically.
                let scope = Scope::child(Rc::clone(env));
                // The call-frame evaluator, built up front so default-parameter expressions
                // evaluate in this *call* scope — where earlier params are already bound (so a
                // default like `b = a + 1` can see `a`) and free vars still resolve lexically
                // through the closure's `env`. Evaluating against `self.scope` (the call *site*)
                // would see neither, matching the bytecode VM's ArgMissing prologue, which runs
                // defaults in the frame after the supplied args are bound.
                let mut eval = Evaluator {
                    scope: Rc::clone(&scope),
                    module_cache: Rc::clone(&self.module_cache),
                    current_dir: RefCell::new(self.current_dir.borrow().clone()),
                    virtual_builtins: Rc::clone(&self.virtual_builtins),
            string_builder: Rc::clone(&self.string_builder),
                    call_depth: Rc::clone(&self.call_depth),
                    max_call_depth: self.max_call_depth,
                };
                {
                    let mut s = scope.borrow_mut();
                    for (i, formal) in formals.iter().enumerate() {
                        let val = match args.get(i) {
                            Some(v) => v.clone(),
                            None => {
                                let def = match formal {
                                    FunParam::Simple(tp) => tp.default.as_ref(),
                                    FunParam::Destructure { default, .. } => default.as_ref(),
                                };
                                if let Some(default_expr) = def {
                                    drop(s);
                                    let default_val = eval.eval_expr(default_expr)?;
                                    s = scope.borrow_mut();
                                    default_val
                                } else {
                                    Value::Null
                                }
                            }
                        };
                        match formal {
                            FunParam::Simple(tp) => {
                                s.set(Arc::clone(&tp.name), val, true);
                            }
                            FunParam::Destructure { pattern, .. } => {
                                drop(s);
                                Self::bind_destruct_pattern_scoped(&scope, pattern, &val, true)?;
                                s = scope.borrow_mut();
                            }
                        }
                    }
                    if let Some(ref rest_name) = rest_param {
                        let rest_vals: Vec<Value> =
                            args.iter().skip(formals.len()).cloned().collect();
                        s.set(
                            Arc::clone(rest_name),
                            Value::Array(Rc::new(RefCell::new(rest_vals))),
                            true,
                        );
                    }
                }
                // Grow the native stack on demand so deep (non-tail) recursion doesn't overflow
                // the OS thread stack — same idea as the bytecode VM's `stacker::maybe_grow` around
                // recursive `run_chunk` (vm.rs:1138). Without it the tree-walker aborts (SIGABRT,
                // "stack overflow") on deep recursion, which the cross-backend parity run surfaced
                // on `recursion_stress`. This is the recursion ACCUMULATOR (every user-function call
                // lands here); the per-element HOF path (`call_with_scope`) is deliberately NOT
                // guarded — it never nests deeply, so it avoids the per-call check on hot map/filter.
                //
                // Red zone = 1 MiB, NOT the VM's 128 KiB: one tree-walker recursion level spans a
                // long eval chain (eval_statement → eval_expr(if) → eval_expr(binary) → eval_expr(call)
                // → eval_call_args → call_func → …), each frame large — far more per level than the
                // VM's single `run_chunk` re-entry. 128 KiB is smaller than one level's chain, so the
                // stack overflows BETWEEN checks; 1 MiB comfortably covers a level (verified to depth
                // 20000 in both debug and release). 16 MiB segments keep grow frequency low.
                // #381: bound recursion with a catchable `RangeError` instead of letting `stacker`
                // grow the stack toward OOM/abort. The counter is shared (`Rc`) across every call
                // frame, so it measures true nesting depth; it is decremented on the way out (both
                // the Ok and Err paths) via the explicit `set` below so a caught throw doesn't leak
                // depth.
                let depth = eval.call_depth.get() + 1;
                if depth > eval.max_call_depth {
                    let err = crate::natives::range_error_construct(&[Value::String(
                        "Maximum call stack size exceeded".into(),
                    )])
                    .unwrap_or(Value::Null);
                    return Err(EvalError::Throw(err));
                }
                eval.call_depth.set(depth);
                let body_result = {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        stacker::maybe_grow(1024 * 1024, 16 * 1024 * 1024, || {
                            eval.eval_statement(body)
                        })
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        eval.eval_statement(body)
                    }
                };
                eval.call_depth.set(depth - 1);
                match body_result {
                    Ok(v) => Ok(v),
                    Err(EvalError::Return(v)) => Ok(v),
                    Err(EvalError::Throw(v)) => Err(EvalError::Throw(v)),
                    Err(EvalError::Error(s)) => Err(EvalError::Error(s)),
                    Err(EvalError::Break) => {
                        Err(EvalError::Error("break outside loop".to_string()))
                    }
                    Err(EvalError::Continue) => {
                        Err(EvalError::Error("continue outside loop".to_string()))
                    }
                }
            }
            _ => Err(EvalError::Error("Not a function".to_string())),
        }
    }

    #[cfg(feature = "http")]
    fn run_promise_method(
        &self,
        promise_ref: &crate::promise::PromiseRef,
        method: &str,
        args: &[Value],
    ) -> Result<Value, EvalError> {
        match method {
            "then" => {
                self.run_promise_then_core(promise_ref, args.first().cloned(), args.get(1).cloned())
            }
            "catch" => self.run_promise_then_core(promise_ref, None, args.first().cloned()),
            "finally" => self.run_promise_finally(promise_ref, args.first().cloned()),
            _ => Err(EvalError::Error(format!(
                "Unknown promise method: {}",
                method
            ))),
        }
    }

    #[cfg(feature = "http")]
    fn run_promise_finally(
        &self,
        promise_ref: &crate::promise::PromiseRef,
        on_finally: Option<Value>,
    ) -> Result<Value, EvalError> {
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (resolve, reject) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        let state = &promise_ref.state;
        {
            let s = state.borrow();
            match &*s {
                crate::promise::PromiseState::Fulfilled(v) => {
                    let v = v.clone();
                    drop(s);
                    if let Some(ref f) = on_finally {
                        let _ = self.call_func(f, &[]);
                    }
                    crate::promise::settle_promise(&resolve, v, true).map_err(EvalError::Error)?;
                }
                crate::promise::PromiseState::Rejected(v) => {
                    let v = v.clone();
                    drop(s);
                    if let Some(ref f) = on_finally {
                        let _ = self.call_func(f, &[]);
                    }
                    crate::promise::settle_promise(&reject, v, false).map_err(EvalError::Error)?;
                }
                crate::promise::PromiseState::Pending { .. } => {
                    let reaction = if let Some(ref f) = on_finally {
                        crate::promise::Reaction::Finally(f.clone(), resolve, reject)
                    } else {
                        crate::promise::Reaction::Then(None, None, resolve, reject)
                    };
                    crate::promise::add_reaction(state, reaction);
                }
            }
        }
        Ok(promise)
    }

    #[cfg(feature = "http")]
    fn run_promise_then_core(
        &self,
        promise_ref: &crate::promise::PromiseRef,
        on_fulfilled: Option<Value>,
        on_rejected: Option<Value>,
    ) -> Result<Value, EvalError> {
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (resolve, reject) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        let state = &promise_ref.state;
        {
            let s = state.borrow();
            match &*s {
                crate::promise::PromiseState::Fulfilled(v) => {
                    let v = v.clone();
                    drop(s);
                    let result = if let Some(ref h) = on_fulfilled {
                        self.call_func(h, &[v])
                    } else {
                        Ok(v)
                    };
                    match result {
                        Ok(val) => {
                            crate::promise::settle_promise(&resolve, val, true)
                                .map_err(EvalError::Error)?;
                        }
                        Err(EvalError::Throw(val)) => {
                            crate::promise::settle_promise(&reject, val, false)
                                .map_err(EvalError::Error)?;
                        }
                        Err(e) => return Err(e),
                    }
                }
                crate::promise::PromiseState::Rejected(v) => {
                    let v = v.clone();
                    drop(s);
                    let result = if let Some(ref h) = on_rejected {
                        self.call_func(h, &[v.clone()])
                    } else {
                        Err(EvalError::Throw(v))
                    };
                    match result {
                        Ok(val) => {
                            crate::promise::settle_promise(&resolve, val, true)
                                .map_err(EvalError::Error)?;
                        }
                        Err(EvalError::Throw(val)) => {
                            crate::promise::settle_promise(&reject, val, false)
                                .map_err(EvalError::Error)?;
                        }
                        Err(e) => return Err(e),
                    }
                }
                crate::promise::PromiseState::Pending { .. } => {
                    crate::promise::add_reaction(
                        state,
                        crate::promise::Reaction::Then(
                            on_fulfilled,
                            on_rejected,
                            resolve.clone(),
                            reject.clone(),
                        ),
                    );
                }
            }
        }
        Ok(promise)
    }

    #[cfg(feature = "timers")]
    fn run_timer_builtin(&self, name: &str, args: &[Value]) -> Result<Value, EvalError> {
        let callback = args
            .first()
            .ok_or_else(|| EvalError::Error(format!("{} requires a callback", name)))?
            .clone();
        let delay_ms = args
            .get(1)
            .and_then(|v| v.as_number())
            .unwrap_or(0.0)
            .max(0.0) as u64;
        let extra_args: Vec<Value> = args.iter().skip(2).cloned().collect();

        let id = match name {
            "setTimeout" => crate::timers::setTimeout(callback, extra_args, delay_ms),
            "setInterval" => crate::timers::setInterval(callback, extra_args, delay_ms),
            _ => return Err(EvalError::Error(format!("Unknown timer: {}", name))),
        };
        Ok(Value::Number(id as f64))
    }

    #[cfg(feature = "timers")]
    fn clear_timeout_native(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Number(n)) = args.first() {
            crate::timers::clearTimer(*n as u64);
        }
        Ok(Value::Null)
    }

    #[cfg(feature = "timers")]
    fn clear_interval_native(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Number(n)) = args.first() {
            crate::timers::clearTimer(*n as u64);
        }
        Ok(Value::Null)
    }

    /// Run all due timer callbacks. Called after the script completes so setTimeout/setInterval
    /// callbacks run without blocking the main script. Loops until no timers are due.
    #[cfg(feature = "timers")]
    pub fn run_timer_phase(&mut self) -> Result<(), String> {
        const MAX_ITERATIONS: u32 = 1_000_000; // avoid infinite loop if setInterval never cleared
        let mut iterations = 0;
        while crate::timers::has_pending_timers() && iterations < MAX_ITERATIONS {
            iterations += 1;
            let due = crate::timers::take_due_timers();
            if due.is_empty() {
                // None due yet; sleep until next timer
                let next = crate::timers::next_due_instant();
                if let Some(instant) = next {
                    let now = std::time::Instant::now();
                    if instant > now {
                        std::thread::sleep(instant.duration_since(now));
                    }
                }
                continue;
            }
            for (id, callback, args, interval_ms) in due {
                self.call_func(&callback, &args).map_err(|e| match e {
                    EvalError::Error(s) => s,
                    EvalError::Throw(v) => v.to_string(),
                    _ => "timer callback error".to_string(),
                })?;
                if interval_ms > 0 {
                    crate::timers::re_register_interval(id, callback, args, interval_ms);
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "http")]
    fn run_http_server(&self, args: &[Value]) -> Result<Value, EvalError> {
        use std::io::Write;

        let port = match args.first() {
            Some(Value::Number(n)) => *n as u16,
            _ => return Err(EvalError::Error("serve requires a port number".to_string())),
        };

        let max_requests: Option<usize> = args.get(2).and_then(|v| match v {
            Value::Number(n) if *n >= 1.0 => Some(*n as usize),
            _ => None,
        });

        let handler = match args.get(1) {
            Some(f @ Value::Function { .. }) | Some(f @ Value::Native(_)) => f.clone(),
            _ => {
                return Err(EvalError::Error(
                    "serve requires a handler function".to_string(),
                ))
            }
        };

        let server = crate::http::create_server(port).map_err(EvalError::Error)?;
        println!("Server listening on http://0.0.0.0:{}", port);

        if max_requests == Some(1) {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{}", port))
                {
                    let _ = stream.write_all(
                        b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    );
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                }
            });
        }

        for (count, mut request) in server.incoming_requests().enumerate() {
            let req_value = crate::http::request_to_value(&mut request);

            let response_value = match self.call_func(&handler, &[req_value]) {
                Ok(v) => v,
                Err(EvalError::Throw(v)) => {
                    let mut err_obj: PropMap = PropMap::with_capacity(2);
                    err_obj.insert(Arc::from("status"), Value::Number(500.0));
                    err_obj.insert(Arc::from("body"), Value::String(v.to_string().into()));
                    Value::object(err_obj)
                }
                Err(e) => {
                    let mut err_obj: PropMap = PropMap::with_capacity(2);
                    err_obj.insert(Arc::from("status"), Value::Number(500.0));
                    err_obj.insert(Arc::from("body"), Value::String(e.to_string().into()));
                    Value::object(err_obj)
                }
            };

            if let Some((status, headers, file_path)) =
                crate::http::extract_file_from_response(&response_value)
            {
                crate::http::send_file_response(request, status, headers, file_path);
            } else {
                let (status, headers, body) = crate::http::value_to_response(&response_value);
                crate::http::send_response(request, status, headers, body);
            }
            if max_requests.map(|m| count + 1 >= m).unwrap_or(false) {
                break;
            }
        }

        Ok(Value::Null)
    }

    fn eval_call_args(&self, args: &[tishlang_ast::CallArg]) -> Result<Vec<Value>, EvalError> {
        let mut result = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                tishlang_ast::CallArg::Expr(e) => {
                    result.push(self.eval_expr(e)?);
                }
                tishlang_ast::CallArg::Spread(e) => {
                    let spread_val = self.eval_expr(e)?;
                    if let Value::Array(arr) = &spread_val {
                        result.extend(arr.borrow().iter().cloned());
                    } else if let Some(items) = self.drain_eval_iterator(&spread_val) {
                        // Spread a Map/Set iterator into call args (`f(...m.values())`).
                        result.extend(items);
                    }
                }
            }
        }
        Ok(result)
    }

    fn bind_destruct_pattern_scoped(
        scope: &Rc<RefCell<Scope>>,
        pattern: &tishlang_ast::DestructPattern,
        value: &Value,
        mutable: bool,
    ) -> Result<(), EvalError> {
        match pattern {
            tishlang_ast::DestructPattern::Array(elements) => {
                let arr = match value {
                    Value::Array(a) => a.borrow().clone(),
                    _ => {
                        return Err(EvalError::Error(
                            "Cannot destructure non-array value".to_string(),
                        ))
                    }
                };

                for (i, elem) in elements.iter().enumerate() {
                    if let Some(el) = elem {
                        match el {
                            tishlang_ast::DestructElement::Ident(name, _) => {
                                let val = arr.get(i).cloned().unwrap_or(Value::Null);
                                scope.borrow_mut().set(Arc::clone(name), val, mutable);
                            }
                            tishlang_ast::DestructElement::Pattern(nested) => {
                                let val = arr.get(i).cloned().unwrap_or(Value::Null);
                                Self::bind_destruct_pattern_scoped(scope, nested, &val, mutable)?;
                            }
                            tishlang_ast::DestructElement::Rest(name, _) => {
                                let rest: Vec<Value> = arr.iter().skip(i).cloned().collect();
                                scope.borrow_mut().set(
                                    Arc::clone(name),
                                    Value::Array(Rc::new(RefCell::new(rest))),
                                    mutable,
                                );
                                break;
                            }
                        }
                    }
                }
            }
            tishlang_ast::DestructPattern::Object(props) => {
                let obj = match value {
                    Value::Object(o) => o.borrow().clone(),
                    _ => {
                        return Err(EvalError::Error(
                            "Cannot destructure non-object value".to_string(),
                        ))
                    }
                };

                for prop in props {
                    let val = obj
                        .strings
                        .get(prop.key.as_ref())
                        .cloned()
                        .unwrap_or(Value::Null);
                    match &prop.value {
                        tishlang_ast::DestructElement::Ident(name, _) => {
                            scope.borrow_mut().set(Arc::clone(name), val, mutable);
                        }
                        tishlang_ast::DestructElement::Pattern(nested) => {
                            Self::bind_destruct_pattern_scoped(scope, nested, &val, mutable)?;
                        }
                        tishlang_ast::DestructElement::Rest(_, _) => {
                            return Err(EvalError::Error(
                                "Rest not supported in object destructuring".to_string(),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn bind_destruct_pattern(
        &mut self,
        pattern: &tishlang_ast::DestructPattern,
        value: &Value,
        mutable: bool,
    ) -> Result<(), EvalError> {
        Self::bind_destruct_pattern_scoped(&self.scope, pattern, value, mutable)
    }

    /// `String.prototype.lastIndexOf` (interpreter). Kept as a helper so dispatch cannot fall
    /// through to [`Self::get_prop`] + [`Self::call_func`] for string receivers.
    fn string_last_index_of_eval(arg_vals: &[Value], receiver: &Arc<str>) -> Value {
        let search = match arg_vals.first() {
            Some(Value::String(ss)) => ss.as_ref(),
            _ => return Value::Number(-1.0),
        };
        let position_core: tishlang_core::Value = match arg_vals.get(1) {
            None => tishlang_core::Value::Number(f64::INFINITY),
            Some(Value::Null) => tishlang_core::Value::Null,
            Some(Value::Number(n)) => tishlang_core::Value::Number(*n),
            Some(Value::Bool(b)) => tishlang_core::Value::Bool(*b),
            Some(_) => tishlang_core::Value::Number(0.0),
        };
        let out =
            tishlang_builtins::string::last_index_of_str(receiver.as_ref(), search, &position_core);
        match out {
            tishlang_core::Value::Number(n) => Value::Number(n),
            _ => Value::Number(-1.0),
        }
    }

    /// Drain a JS iterator object — one whose `next()` is a bridged core fn (`CoreFn`)
    /// returning `{ value, done }`, e.g. a Map/Set iterator from `.values()` / `.keys()` /
    /// `.entries()` — into a `Vec` by calling `next()` until `done`. Returns `None` when `v`
    /// is not such an object. Shared by `for…of` and spread so both treat iterators like JS.
    fn drain_eval_iterator(&self, v: &Value) -> Option<Vec<Value>> {
        if !matches!(v, Value::Object(_)) {
            return None;
        }
        // Fast path: tish's Map/Set iterators expose `__drain__`, returning all remaining items as
        // one array — skips the per-element bridge + `{ value, done }` alloc of the generic loop.
        if let Ok(Value::CoreFn(drain)) = self.get_prop(v, "__drain__") {
            if let Value::Array(arr) = crate::value_convert::core_to_eval(drain.call(&[])) {
                return Some(arr.borrow().clone());
            }
        }
        let Ok(Value::CoreFn(next)) = self.get_prop(v, "next") else {
            return None;
        };
        let mut out = Vec::new();
        loop {
            let res = crate::value_convert::core_to_eval(next.call(&[]));
            let done = self
                .get_prop(&res, "done")
                .map(|x| x.is_truthy())
                .unwrap_or(true);
            if done {
                break;
            }
            out.push(self.get_prop(&res, "value").unwrap_or(Value::Null));
        }
        Some(out)
    }

    fn get_prop(&self, obj: &Value, key: &str) -> Result<Value, String> {
        match obj {
            Value::Object(map) => {
                // `Set`/`Map` instances expose a computed `.size` via a hidden `SizeProbe` opaque
                // (shared, not copied, across the value bridge — so it reflects the live store).
                if key == "size" {
                    if let Some(Value::Opaque(op)) =
                        map.borrow().strings.get(tishlang_builtins::collections::SIZE_SLOT)
                    {
                        if let Some(n) = tishlang_builtins::collections::size_probe_len(op.as_ref()) {
                            return Ok(Value::Number(n));
                        }
                    }
                }
                Ok(map.borrow().strings.get(key).cloned().unwrap_or(Value::Null))
            }
            Value::Array(arr) => {
                if key == "length" {
                    Ok(Value::Number(arr.borrow().len() as f64))
                } else if let Ok(idx) = key.parse::<usize>() {
                    Ok(arr.borrow().get(idx).cloned().unwrap_or(Value::Null))
                } else {
                    Ok(Value::Null)
                }
            }
            Value::String(s) => {
                if key == "length" {
                    Ok(Value::Number(char_count_cached(s) as f64))
                } else {
                    Ok(Value::Null)
                }
            }
            #[cfg(feature = "http")]
            Value::Promise(promise_ref) => match key {
                "then" => Ok(Value::BoundPromiseMethod(
                    promise_ref.clone(),
                    Arc::from("then"),
                )),
                "catch" => Ok(Value::BoundPromiseMethod(
                    promise_ref.clone(),
                    Arc::from("catch"),
                )),
                "finally" => Ok(Value::BoundPromiseMethod(
                    promise_ref.clone(),
                    Arc::from("finally"),
                )),
                _ => Ok(Value::Null),
            },
            #[cfg(feature = "http")]
            Value::CorePromise(_) => Ok(Value::Null),
            #[cfg(feature = "http")]
            Value::PromiseConstructor => match key {
                "resolve" => Ok(Value::Native(Self::promise_resolve)),
                "reject" => Ok(Value::Native(Self::promise_reject)),
                "all" => Ok(Value::Native(Self::promise_all)),
                "race" => Ok(Value::Native(Self::promise_race)),
                "any" => Ok(Value::Native(Self::promise_any)),
                "allSettled" => Ok(Value::Native(Self::promise_all_settled)),
                "spawn" => Ok(Value::Native(Self::promise_spawn_interp)),
                _ => Ok(Value::Null),
            },
            Value::Opaque(o) => {
                if o.get_method(key).is_some() {
                    Ok(Value::OpaqueMethod(Arc::clone(o), Arc::from(key)))
                } else {
                    Ok(Value::Null)
                }
            }
            #[cfg(feature = "regex")]
            Value::RegExp(re) => {
                let re = re.borrow();
                match key {
                    "source" => Ok(Value::String(re.source.clone().into())),
                    "flags" => Ok(Value::String(re.flags_string().into())),
                    "lastIndex" => Ok(Value::Number(re.last_index as f64)),
                    "global" => Ok(Value::Bool(re.flags.global)),
                    "ignoreCase" => Ok(Value::Bool(re.flags.ignore_case)),
                    "multiline" => Ok(Value::Bool(re.flags.multiline)),
                    "dotAll" => Ok(Value::Bool(re.flags.dot_all)),
                    "unicode" => Ok(Value::Bool(re.flags.unicode)),
                    "sticky" => Ok(Value::Bool(re.flags.sticky)),
                    _ => Ok(Value::Null),
                }
            }
            // Reading a property of the nullish value throws a catchable `TypeError`, matching the
            // bytecode VM (`get_member`'s `_` arm), cranelift/wasi, and node — not a silent `null`.
            // The tree-walker used to fall through to `_ => Ok(Value::Null)`, so `null.length` read
            // back as `null` on interp while every other backend threw (a pure interp≠vm bug).
            Value::Null => Err(format!("Cannot read property '{}' of null", key)),
            _ => Ok(Value::Null),
        }
    }

    fn get_index(&self, obj: &Value, index: &Value) -> Result<Value, String> {
        match obj {
            Value::Array(arr) => {
                let idx = match index {
                    Value::Number(n) => *n as usize,
                    _ => return Ok(Value::Null),
                };
                Ok(arr.borrow().get(idx).cloned().unwrap_or(Value::Null))
            }
            // `str[i]` returns the character at index `i` (issue #17). The VM already does
            // this; the interpreter previously fell through to `null`, a silent divergence.
            // Out-of-bounds / negative / non-integer indices yield tish's nullish value.
            Value::String(s) => {
                let idx = match index {
                    Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 => *n as usize,
                    _ => return Ok(Value::Null),
                };
                Ok(nth_char_cached(s, idx)
                    .map(|c| Value::String(c.to_string().into()))
                    .unwrap_or(Value::Null))
            }
            Value::Object(_) => Ok(eval_object_get(obj, index).unwrap_or(Value::Null)),
            #[cfg(feature = "http")]
            Value::Promise(_) | Value::CorePromise(_) => {
                let key = match index {
                    Value::String(s) => s.as_ref(),
                    _ => return Ok(Value::Null),
                };
                self.get_prop(obj, key)
            }
            // Indexing the nullish value throws a catchable `TypeError` (like `get_prop` above and
            // the VM/cranelift/wasi/node) rather than silently reading back `null`.
            Value::Null => Err(format!("Cannot read property '{}' of null", index)),
            _ => Ok(Value::Null),
        }
    }

    fn json_parse(s: &str) -> Value {
        // Delegate to the shared, spec-correct parser in `tishlang_core` and convert into
        // interpreter Values — one source of truth with the VM/native/cranelift/wasi backends.
        // The previous hand-rolled parser depth-counted `{}`/`[]` brackets WITHOUT skipping string
        // contents, so a nested value whose string held `}`/`]`/`{`/`[` (e.g. `{"a":{"s":"}"}}`)
        // mis-sliced and the whole parse failed to `null` where Node/VM succeed; it also re-scanned
        // each nested value O(n^2). The core parser is string-aware and builds insertion-ordered
        // PropMaps. Invalid input still yields `null` (unchanged interpreter behavior).
        match tishlang_core::json_parse(s) {
            Ok(core) => crate::value_convert::core_to_eval(core),
            Err(_) => Value::Null,
        }
    }

    /// #381 — ancestor-guarded like `tishlang_core::json_stringify_into_guarded`: a back-edge to an
    /// ancestor (`a.self = a`) is a cycle and returns `Err(())` instead of recursing forever (this
    /// was an unguarded native-stack overflow → uncatchable abort; core's guard from #389 never
    /// covered the interpreter's own stringifier). Ancestor-path only, not all-visited: a node
    /// repeated across sibling branches is a legal DAG and must serialize twice.
    fn json_stringify_value(v: &Value, ancestors: &mut Vec<*const ()>) -> Result<String, ()> {
        Ok(match v {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => {
                // Format numbers exactly like the VM/native path (shared helper) so JSON output
                // matches across backends and Node — Rust's `{}` Display diverges (e.g. `1e21`,
                // `1e-7`, `-0`). #180.
                let mut s = String::new();
                tishlang_core::write_json_number(&mut s, *n);
                s
            }
            Value::String(s) => format!(
                "\"{}\"",
                s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t")
            ),
            Value::Array(arr) => {
                let ptr = Rc::as_ptr(arr) as *const ();
                if ancestors.contains(&ptr) {
                    return Err(());
                }
                ancestors.push(ptr);
                let borrowed = arr.borrow();
                let mut inner: Vec<String> = Vec::with_capacity(borrowed.len());
                for item in borrowed.iter() {
                    inner.push(Self::json_stringify_value(item, ancestors)?);
                }
                drop(borrowed);
                ancestors.pop();
                format!("[{}]", inner.join(","))
            }
            Value::Object(map) => {
                let ptr = Rc::as_ptr(map) as *const ();
                if ancestors.contains(&ptr) {
                    return Err(());
                }
                ancestors.push(ptr);
                // Insertion order (PropMap is an IndexMap) — matches JS/Node and the
                // VM/rust backends. No key sort.
                let borrowed = map.borrow();
                let mut entries: Vec<String> = Vec::with_capacity(borrowed.strings.len());
                for (k, v) in borrowed.strings.iter() {
                    entries.push(format!(
                        "\"{}\":{}",
                        k.replace('\\', "\\\\").replace('"', "\\\""),
                        Self::json_stringify_value(v, ancestors)?
                    ));
                }
                drop(borrowed);
                ancestors.pop();
                format!("{{{}}}", entries.join(","))
            }
            Value::Symbol(_) => "null".to_string(),
            Value::Function { .. } | Value::Native(_) => "null".to_string(),
            #[cfg(feature = "http")]
            Value::CorePromise(_) => "null".to_string(),
            Value::CoreFn(_) => "null".to_string(),
            #[cfg(feature = "http")]
            Value::Serve
            | Value::Promise(_)
            | Value::PromiseResolver(_)
            | Value::PromiseConstructor
            | Value::BoundPromiseMethod(_, _) => "null".to_string(),
            #[cfg(feature = "timers")]
            Value::TimerBuiltin(_) => "null".to_string(),
            #[cfg(feature = "regex")]
            Value::RegExp(_) => "null".to_string(),
            Value::Opaque(_) | Value::OpaqueMethod(_, _) => "null".to_string(),
        })
    }

    // Static native wrapper functions (these need to be fn pointers, not closures with &self)
    fn json_parse_native(args: &[Value]) -> Result<Value, String> {
        let s = args.first().map(|v| v.to_string()).unwrap_or_default();
        Ok(Self::json_parse(&s))
    }

    fn json_stringify_native(args: &[Value]) -> Result<Value, String> {
        let v = args.first().cloned().unwrap_or(Value::Null);
        let mut ancestors: Vec<*const ()> = Vec::new();
        match Self::json_stringify_value(&v, &mut ancestors) {
            Ok(s) => Ok(Value::String(s.into())),
            // Surfaced by `call_func` as `EvalError::Error`, which the Try handler boxes as
            // `{ name: "TypeError", message }` — node-identical for the circular case (#381).
            Err(()) => Err("Converting circular structure to JSON".to_string()),
        }
    }

    fn object_keys(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(obj)) = args.first() {
            let keys: Vec<Value> = obj
                .borrow()
                .strings
                .keys()
                .map(|k| Value::String(Arc::clone(k)))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(keys))))
        } else {
            Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
        }
    }

    fn object_values(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(obj)) = args.first() {
            let values: Vec<Value> = obj.borrow().strings.values().cloned().collect();
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        } else {
            Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
        }
    }

    fn object_entries(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(obj)) = args.first() {
            let entries: Vec<Value> = obj
                .borrow()
                .strings
                .iter()
                .map(|(k, v)| {
                    Value::Array(Rc::new(RefCell::new(vec![
                        Value::String(Arc::clone(k)),
                        v.clone(),
                    ])))
                })
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(entries))))
        } else {
            Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
        }
    }

    fn object_assign(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(target)) = args.first() {
            let mut t = target.borrow_mut();
            for src in args.iter().skip(1) {
                if let Value::Object(src_obj) = src {
                    let s = src_obj.borrow();
                    for (k, v) in s.strings.iter() {
                        t.strings.insert(Arc::clone(k), v.clone());
                    }
                    if let Some(ref sm) = s.symbols {
                        if t.symbols.is_none() {
                            t.symbols = Some(AHashMap::default());
                        }
                        let tm = t.symbols.as_mut().unwrap();
                        for (id, v) in sm.iter() {
                            tm.insert(*id, v.clone());
                        }
                    }
                }
            }
            drop(t);
            Ok(args.first().cloned().unwrap())
        } else {
            Ok(Value::Null)
        }
    }

    fn object_from_entries(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Array(arr)) = args.first() {
            let mut map = PropMap::default();
            for entry in arr.borrow().iter() {
                if let Value::Array(pair) = entry {
                    let pair = pair.borrow();
                    if let (Some(key), Some(value)) = (pair.first(), pair.get(1)) {
                        let key_str: Arc<str> = key.to_string().into();
                        map.insert(key_str, value.clone());
                    }
                }
            }
            Ok(Value::object(map))
        } else {
            Ok(Value::object(PropMap::default()))
        }
    }

    #[cfg(feature = "regex")]
    fn regexp_constructor_native(args: &[Value]) -> Result<Value, String> {
        crate::regex::regexp_constructor(args)
    }

    #[cfg(feature = "http")]
    fn promise_resolve(args: &[Value]) -> Result<Value, String> {
        let x = args.first().cloned().unwrap_or(Value::Null);
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (resolve, _) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        crate::promise::settle_promise(&resolve, x, true)?;
        Ok(promise)
    }

    #[cfg(feature = "http")]
    fn promise_reject(args: &[Value]) -> Result<Value, String> {
        let r = args.first().cloned().unwrap_or(Value::Null);
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (_, reject) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        crate::promise::settle_promise(&reject, r, false)?;
        Ok(promise)
    }

    #[cfg(feature = "http")]
    fn promise_all(args: &[Value]) -> Result<Value, String> {
        let iterable = args
            .first()
            .ok_or_else(|| "Promise.all requires an iterable".to_string())?;
        let values: Vec<Value> = match iterable {
            Value::Array(arr) => arr.borrow().clone(),
            Value::String(s) => s
                .chars()
                .map(|c| Value::String(c.to_string().into()))
                .collect(),
            _ => return Err("Promise.all requires array or iterable".to_string()),
        };
        let mut results = Vec::with_capacity(values.len());
        for v in values {
            if let Value::Promise(ref p) = v {
                match crate::promise::block_until_settled(p) {
                    crate::promise::PromiseAwaitResult::Fulfilled(x) => results.push(x),
                    crate::promise::PromiseAwaitResult::Rejected(x) => {
                        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
                        let (_, reject) =
                            crate::promise::extract_resolvers(&resolve_val, &reject_val);
                        let _ = crate::promise::settle_promise(&reject, x, false);
                        return Ok(promise);
                    }
                    crate::promise::PromiseAwaitResult::Error(e) => return Err(e),
                }
            } else if let Value::CorePromise(ref p) = v {
                match p.block_until_settled() {
                    Ok(x) => results.push(crate::value_convert::core_to_eval(x)),
                    Err(x) => {
                        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
                        let (_, reject) =
                            crate::promise::extract_resolvers(&resolve_val, &reject_val);
                        let _ = crate::promise::settle_promise(
                            &reject,
                            crate::value_convert::core_to_eval(x),
                            false,
                        );
                        return Ok(promise);
                    }
                }
            } else {
                results.push(v);
            }
        }
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (resolve, _) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        let arr = Value::Array(Rc::new(RefCell::new(results)));
        crate::promise::settle_promise(&resolve, arr, true)?;
        Ok(promise)
    }

    #[cfg(feature = "http")]
    fn promise_race(args: &[Value]) -> Result<Value, String> {
        let iterable = args
            .first()
            .ok_or_else(|| "Promise.race requires an iterable".to_string())?;
        let values: Vec<Value> = match iterable {
            Value::Array(arr) => arr.borrow().clone(),
            Value::String(s) => s
                .chars()
                .map(|c| Value::String(c.to_string().into()))
                .collect(),
            _ => return Err("Promise.race requires array or iterable".to_string()),
        };
        for v in values {
            if let Value::CorePromise(ref p) = v {
                match p.block_until_settled() {
                    Ok(x) => {
                        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
                        let (resolve, _) =
                            crate::promise::extract_resolvers(&resolve_val, &reject_val);
                        crate::promise::settle_promise(
                            &resolve,
                            crate::value_convert::core_to_eval(x),
                            true,
                        )?;
                        return Ok(promise);
                    }
                    Err(x) => {
                        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
                        let (_, reject) =
                            crate::promise::extract_resolvers(&resolve_val, &reject_val);
                        crate::promise::settle_promise(
                            &reject,
                            crate::value_convert::core_to_eval(x),
                            false,
                        )?;
                        return Ok(promise);
                    }
                }
            }
            if let Value::Promise(ref p) = v {
                match crate::promise::block_until_settled(p) {
                    crate::promise::PromiseAwaitResult::Fulfilled(x) => {
                        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
                        let (resolve, _) =
                            crate::promise::extract_resolvers(&resolve_val, &reject_val);
                        crate::promise::settle_promise(&resolve, x, true)?;
                        return Ok(promise);
                    }
                    crate::promise::PromiseAwaitResult::Rejected(x) => {
                        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
                        let (_, reject) =
                            crate::promise::extract_resolvers(&resolve_val, &reject_val);
                        crate::promise::settle_promise(&reject, x, false)?;
                        return Ok(promise);
                    }
                    crate::promise::PromiseAwaitResult::Error(e) => return Err(e),
                }
            }
        }
        Err("Promise.race requires at least one promise".to_string())
    }

    /// Helper: settle a new promise fulfilled with `v` (interp Value).
    #[cfg(feature = "http")]
    fn eval_fulfilled(v: Value) -> Result<Value, String> {
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (resolve, _) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        crate::promise::settle_promise(&resolve, v, true)?;
        Ok(promise)
    }

    /// Helper: settle a new promise rejected with `v` (interp Value).
    #[cfg(feature = "http")]
    fn eval_rejected(v: Value) -> Result<Value, String> {
        let (promise, resolve_val, reject_val) = crate::promise::create_promise();
        let (_, reject) = crate::promise::extract_resolvers(&resolve_val, &reject_val);
        crate::promise::settle_promise(&reject, v, false)?;
        Ok(promise)
    }

    /// Await one interp promise/core-promise/value → `Result<Value, Value>`.
    #[cfg(feature = "http")]
    fn settle_one(v: Value) -> Result<Value, Value> {
        match v {
            Value::Promise(ref p) => match crate::promise::block_until_settled(p) {
                crate::promise::PromiseAwaitResult::Fulfilled(x) => Ok(x),
                crate::promise::PromiseAwaitResult::Rejected(x) => Err(x),
                crate::promise::PromiseAwaitResult::Error(e) => {
                    Err(Value::String(e.into()))
                }
            },
            Value::CorePromise(ref p) => match p.block_until_settled() {
                Ok(x) => Ok(crate::value_convert::core_to_eval(x)),
                Err(x) => Err(crate::value_convert::core_to_eval(x)),
            },
            other => Ok(other),
        }
    }

    /// `Promise.any(iterable)` — first fulfilled wins; rejects with array of reasons if all reject.
    #[cfg(feature = "http")]
    fn promise_any(args: &[Value]) -> Result<Value, String> {
        let iterable = args
            .first()
            .ok_or_else(|| "Promise.any requires an iterable".to_string())?;
        let values: Vec<Value> = match iterable {
            Value::Array(arr) => arr.borrow().clone(),
            _ => return Err("Promise.any requires an array".to_string()),
        };
        let n = values.len();
        if n == 0 {
            return Self::eval_rejected(Value::Array(Rc::new(RefCell::new(vec![]))));
        }
        let mut errors = Vec::with_capacity(n);
        for v in values {
            match Self::settle_one(v) {
                Ok(x) => return Self::eval_fulfilled(x),
                Err(e) => errors.push(e),
            }
        }
        Self::eval_rejected(Value::Array(Rc::new(RefCell::new(errors))))
    }

    /// `Promise.allSettled(iterable)` — always fulfills with array of `{status,value|reason}`.
    #[cfg(feature = "http")]
    fn promise_all_settled(args: &[Value]) -> Result<Value, String> {
        use crate::value::EvalObjectData;
        let iterable = args
            .first()
            .ok_or_else(|| "Promise.allSettled requires an iterable".to_string())?;
        let values: Vec<Value> = match iterable {
            Value::Array(arr) => arr.borrow().clone(),
            _ => return Err("Promise.allSettled requires an array".to_string()),
        };
        let mut out = Vec::with_capacity(values.len());
        for v in values {
            let r = Self::settle_one(v);
            let mut data = EvalObjectData::default();
            match r {
                Ok(x) => {
                    data.strings.insert(std::sync::Arc::from("status"), Value::String("fulfilled".into()));
                    data.strings.insert(std::sync::Arc::from("value"), x);
                }
                Err(e) => {
                    data.strings.insert(std::sync::Arc::from("status"), Value::String("rejected".into()));
                    data.strings.insert(std::sync::Arc::from("reason"), e);
                }
            }
            out.push(Value::Object(Rc::new(RefCell::new(data))));
        }
        Self::eval_fulfilled(Value::Array(Rc::new(RefCell::new(out))))
    }

    /// `Promise.spawn(fn)` — on the interpreter, runs the function synchronously and wraps
    /// the result in an immediate promise. The interpreter uses `Rc<RefCell<…>>` for closures,
    /// which is `!Send`, so we cannot move the function to a background thread here. Real
    /// cross-thread parallelism via spawn is available on the bytecode VM (which uses the
    /// `send-values` / Arc path for the shipped `full` build). For the interpreter, `any` and
    /// `race` over spawn-created promises still work correctly — they just don't run concurrently.
    #[cfg(feature = "http")]
    fn promise_spawn_interp(args: &[Value]) -> Result<Value, String> {
        let callable = match args.first() {
            Some(v @ (Value::Native(_) | Value::Function { .. })) => v.clone(),
            _ => return Err("Promise.spawn: expected a function argument".to_string()),
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match &callable {
                Value::Native(f) => f(&[]).map_err(|e| e.to_string()),
                // Interpreter closures (Value::Function) can't be called from a static native fn
                // (no evaluator state / Rc captures). Use the VM backend for concurrent CPU spawn.
                _ => Err("Promise.spawn: tish closures are not supported on the interpreter backend; use the vm backend (tish run) or pass a native module function".to_string()),
            }
        }));
        match result {
            Ok(Ok(v))  => Self::eval_fulfilled(v),
            Ok(Err(e)) => Self::eval_rejected(Value::String(e.into())),
            Err(_)     => Self::eval_rejected(Value::String("Promise.spawn: task panicked".into())),
        }
    }

    #[cfg(feature = "ws")]
    fn ws_web_socket_native(args: &[Value]) -> Result<Value, String> {
        let mut cv = Vec::new();
        for a in args {
            cv.push(crate::value_convert::eval_to_core(a)?);
        }
        Ok(crate::value_convert::core_to_eval(
            tishlang_runtime::web_socket_client(&cv),
        ))
    }

    #[cfg(feature = "ws")]
    fn ws_server_native(args: &[Value]) -> Result<Value, String> {
        let mut cv = Vec::new();
        for a in args {
            cv.push(crate::value_convert::eval_to_core(a)?);
        }
        Ok(crate::value_convert::core_to_eval(
            tishlang_runtime::web_socket_server_construct(&cv),
        ))
    }

    #[cfg(feature = "ws")]
    fn ws_send_native(args: &[Value]) -> Result<Value, String> {
        let conn = args.first().ok_or("wsSend(conn, data) requires conn")?;
        let conn_core = crate::value_convert::eval_to_core(conn)?;
        let data = args.get(1).map(|v| v.to_string()).unwrap_or_default();
        Ok(Value::Bool(tishlang_runtime::ws_send_native(
            &conn_core, &data,
        )))
    }

    #[cfg(feature = "ws")]
    fn ws_broadcast_native(args: &[Value]) -> Result<Value, String> {
        let mut cv = Vec::new();
        for a in args {
            cv.push(crate::value_convert::eval_to_core(a)?);
        }
        Ok(crate::value_convert::core_to_eval(
            tishlang_runtime::ws_broadcast_native(&cv),
        ))
    }

    #[cfg(feature = "http")]
    fn fetch_native(args: &[Value]) -> Result<Value, String> {
        let mut cv = Vec::new();
        for a in args {
            cv.push(crate::value_convert::eval_to_core(a)?);
        }
        match tishlang_runtime::fetch_promise(cv) {
            tishlang_core::Value::Promise(p) => Ok(Value::CorePromise(p)),
            _ => Err("internal: fetch did not return Promise".into()),
        }
    }

    #[cfg(feature = "http")]
    fn fetch_all_native(args: &[Value]) -> Result<Value, String> {
        let mut cv = Vec::new();
        for a in args {
            cv.push(crate::value_convert::eval_to_core(a)?);
        }
        match tishlang_runtime::fetch_all_promise(cv) {
            tishlang_core::Value::Promise(p) => Ok(Value::CorePromise(p)),
            _ => Err("internal: fetchAll did not return Promise".into()),
        }
    }

    #[cfg(feature = "http")]
    fn eval_await(&self, operand: &Expr) -> Result<Value, EvalError> {
        let val = self.eval_expr(operand)?;
        if let Value::Promise(ref p) = val {
            match crate::promise::block_until_settled(p) {
                crate::promise::PromiseAwaitResult::Fulfilled(v) => Ok(v),
                crate::promise::PromiseAwaitResult::Rejected(v) => Err(EvalError::Throw(v)),
                crate::promise::PromiseAwaitResult::Error(e) => Err(EvalError::Error(e)),
            }
        } else if let Value::CorePromise(ref p) = val {
            match p.block_until_settled() {
                Ok(v) => Ok(crate::value_convert::core_to_eval(v)),
                Err(v) => Err(EvalError::Throw(crate::value_convert::core_to_eval(v))),
            }
        } else {
            Err(EvalError::Error(
                "await requires a Promise (use await fetch(...), await reader.read(), etc.)".into(),
            ))
        }
    }

    #[cfg(not(feature = "http"))]
    fn eval_await(&self, _operand: &Expr) -> Result<Value, EvalError> {
        Err(EvalError::Error(
            "await requires the http feature".to_string(),
        ))
    }
}

#[derive(Debug)]
enum EvalError {
    Return(Value),
    Break,
    Continue,
    Throw(Value),
    Error(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EvalError::Return(_) => write!(f, "return"),
            EvalError::Break => write!(f, "break"),
            EvalError::Continue => write!(f, "continue"),
            EvalError::Throw(v) => write!(f, "{}", v),
            EvalError::Error(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for EvalError {}

#[cfg(test)]
mod recursion_limit_tests_381 {
    use super::Evaluator;
    use tishlang_parser::parse;

    fn run_with_depth(src: &str, max_depth: usize) -> String {
        let program = parse(src).unwrap();
        let mut eval = Evaluator::new();
        eval.max_call_depth = max_depth;
        eval.eval_program(&program).unwrap().to_string()
    }

    #[test]
    fn deep_recursion_throws_catchable_range_error() {
        // Infinite (non-tail) recursion past the limit must throw a CATCHABLE RangeError, not abort:
        // try/catch recovers and the program keeps running.
        let src = "fn rec(n) { return 1 + rec(n + 1) }\n\
                   let name = 'none'\n\
                   try { rec(0) } catch (e) { name = e.name }\n\
                   name";
        assert_eq!(run_with_depth(src, 200), "RangeError");
    }

    #[test]
    fn normal_recursion_is_unaffected() {
        let src = "fn fib(n) { if (n < 2) { return n } return fib(n - 1) + fib(n - 2) }\nfib(12)";
        assert_eq!(run_with_depth(src, 20000), "144");
    }
}

#[cfg(test)]
mod null_member_access_tests {
    // Reading a property or index of the nullish value throws a catchable `TypeError`, matching the
    // bytecode VM / cranelift / wasi / node. The tree-walker used to fall through to `Ok(Null)`, so
    // `null.length` read back as `null` on interp while every other backend threw (a pure interp≠vm
    // divergence). These lock the interpreter to the throwing behavior.
    use super::Evaluator;
    use tishlang_parser::parse;

    fn run(src: &str) -> String {
        let program = parse(src).unwrap();
        let mut eval = Evaluator::new();
        eval.eval_program(&program).unwrap().to_string()
    }

    #[test]
    fn null_property_read_throws_type_error() {
        let src = "let name = 'none'\n\
                   try { let z = null; z.length } catch (e) { name = e.name }\n\
                   name";
        assert_eq!(run(src), "TypeError");
    }

    #[test]
    fn null_index_read_throws_type_error() {
        let src = "let name = 'none'\n\
                   try { let z = null; z[0] } catch (e) { name = e.name }\n\
                   name";
        assert_eq!(run(src), "TypeError");
    }

    #[test]
    fn object_missing_property_still_reads_null() {
        // Guard against over-reach: a MISSING property of a real object is `null`, not a throw.
        let src = "let o = { a: 1 }\nString(o.b === null)";
        assert_eq!(run(src), "true");
    }

    #[test]
    fn array_oob_index_still_reads_null() {
        // Out-of-bounds array index stays nullish (only a null/undefined *receiver* throws).
        let src = "let a = [1, 2, 3]\nString(a[9] === null)";
        assert_eq!(run(src), "true");
    }
}

#[cfg(test)]
mod array_hof_callback_arg_tests {
    // Array HOFs pass the source array as the trailing callback arg (JS `(element, index, array)`,
    // reduce `(acc, element, index, array)`). The inline interp loops snapshot the backing store and
    // drop the borrow before any callback, so a callback that MUTATES the array via the `array` arg
    // (or a capture) can't RefCell-panic — it iterates the pre-call snapshot, matching JS + the shared
    // #382 builtins path.
    use super::Evaluator;
    use tishlang_parser::parse;

    fn run(src: &str) -> String {
        let program = parse(src).unwrap();
        let mut eval = Evaluator::new();
        eval.eval_program(&program).unwrap().to_string()
    }

    #[test]
    fn map_receives_array_as_third_arg() {
        assert_eq!(run("[5, 3, 8].map((x, i, arr) => arr.length).join(',')"), "3,3,3");
    }

    #[test]
    fn reduce_receives_array_as_fourth_arg() {
        assert_eq!(run("[1, 2, 3].reduce((a, x, i, arr) => a + arr.length, 0)"), "9");
    }

    #[test]
    fn callback_may_index_the_array_arg() {
        assert_eq!(run("[10, 20, 30].map((x, i, arr) => arr[i] * 2).join(',')"), "20,40,60");
    }

    #[test]
    fn callback_mutating_the_array_arg_does_not_panic() {
        // Pre-snapshot this RefCell-panicked (map held the borrow across the loop). Now iteration is
        // over the snapshot (3 elements); the 3 pushes land on the live array → final length 6, matching
        // JS forEach semantics (does not visit elements appended during iteration).
        let src = "let a = [1, 2, 3]\na.forEach((x, i, arr) => { arr.push(x) })\na.length";
        assert_eq!(run(src), "6");
    }

    #[test]
    fn one_and_two_arg_callbacks_unchanged() {
        assert_eq!(run("[5, 3, 8].map(x => x * 2).join(',')"), "10,6,16");
        assert_eq!(run("[5, 3, 8].map((x, i) => x + i).join(',')"), "5,4,10");
    }
}
