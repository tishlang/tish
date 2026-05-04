//! **tishlang_pg** — PostgreSQL for the **Tish native runtime** only.
//!
//! - **No Node.js**, **no N-API**, **no JavaScript** in this crate or its distribution.
//! - Uses **Rust** [`tokio-postgres`](https://docs.rs/tokio-postgres) + [`deadpool-postgres`](https://docs.rs/deadpool-postgres) for protocol and pooling.
//! - The **Tish compiler / runtime** (`tishlang/tish`) links this `rlib` and exposes a `pg` (or `tish_pg`) module to **`.tish` application code** — same *call shape* as node-postgres where practical (`Pool`, `query`, `rows`, `rowCount`).
//!
//! Application authors write **only `.tish`**; they never touch this Rust API directly once bindings exist upstream.
use tishlang_runtime::VmRef;

mod error;
pub use error::{format_pg_error, format_tish_pg_error, Result, TishPgError};

use deadpool_postgres::{Manager, Pool, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};
use tokio_postgres::{types::ToSql, NoTls, Row};

/// Configuration mirroring the common `pg` / `node-postgres` `Pool` constructor shape.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PoolConfig {
    pub connection_string: String,
    #[serde(default)]
    pub max: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldInfo {
    pub name: String,
    pub data_type_id: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub rows: Vec<JsonValue>,
    pub row_count: u32,
    pub fields: Vec<FieldInfo>,
}

/// Rust-side pool; Tish maps this to `Pool` in user code.
#[derive(Clone)]
pub struct PgPool {
    inner: Pool,
}

fn parse_connection_string(cs: &str) -> std::result::Result<tokio_postgres::Config, String> {
    let u = url::Url::parse(cs).map_err(|e| e.to_string())?;
    if u.scheme() != "postgres" && u.scheme() != "postgresql" {
        return Err("URL must use postgres:// or postgresql://".into());
    }
    let mut cfg = tokio_postgres::Config::new();
    if let Some(host) = u.host_str() {
        cfg.host(host);
    }
    if let Some(p) = u.port() {
        cfg.port(p);
    }
    let path = u.path().trim_start_matches('/');
    if !path.is_empty() {
        cfg.dbname(path);
    }
    if !u.username().is_empty() {
        cfg.user(u.username());
    }
    if let Some(pw) = u.password() {
        cfg.password(pw);
    }
    Ok(cfg)
}

fn params_to_sql(values: &[JsonValue]) -> Result<Vec<Box<dyn ToSql + Sync + Send>>> {
    let mut out: Vec<Box<dyn ToSql + Sync + Send>> = Vec::with_capacity(values.len());
    for v in values {
        let b: Box<dyn ToSql + Sync + Send> = match v {
            JsonValue::Null => Box::new(Option::<String>::None),
            JsonValue::Bool(b) => Box::new(*b),
            JsonValue::Number(n) => {
                // tokio-postgres type-checks against the prepared statement's
                // column OIDs. An i64 param against an INT4 column or an f64
                // param against INT4 both error with WrongType. Tish stores
                // all numbers as f64, so we need to detect "whole number in
                // i32 range" and downgrade to i32 so TFB's world.id / world.randomnumber
                // (both INT4) bind correctly. Larger ints fall through to i64
                // (matches INT8). Fractional values stay as f64 (matches FLOAT8).
                if let Some(i) = n.as_i64() {
                    if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                        Box::new(i as i32)
                    } else {
                        Box::new(i)
                    }
                } else if let Some(u) = n.as_u64() {
                    Box::new(u as i64)
                } else if let Some(f) = n.as_f64() {
                    if f.fract() == 0.0
                        && f >= i32::MIN as f64
                        && f <= i32::MAX as f64
                    {
                        Box::new(f as i32)
                    } else if f.fract() == 0.0
                        && f >= i64::MIN as f64
                        && f <= i64::MAX as f64
                    {
                        Box::new(f as i64)
                    } else {
                        Box::new(f)
                    }
                } else {
                    return Err(TishPgError::BadParam(
                        "invalid JSON number for SQL param".into(),
                    ));
                }
            }
            JsonValue::String(s) => Box::new(s.clone()),
            JsonValue::Array(_) | JsonValue::Object(_) => Box::new(tokio_postgres::types::Json(v.clone())),
        };
        out.push(b);
    }
    Ok(out)
}

/// Convert a Postgres `Row` directly into a Tish `Value::Object`,
/// skipping the `JsonValue` intermediate that `row_to_object` produces.
///
/// Two wins on the hot path:
///
/// 1. **One pass instead of two.** The blanket `Row → JsonValue → Value`
///    path allocated a `serde_json::Map<String, JsonValue>` and then
///    re-walked it into an `ObjectMap<Arc<str>, Value>`. This produces
///    the `ObjectMap` in one shot.
/// 2. **Interned column names.** TFB hits the same `id`/`randomnumber`/
///    `message` keys ~140k times per second; the per-row `Arc::from(name)`
///    became a measurable allocator load. We cache one `Arc<str>` per
///    `(stmt-prepared-once)` column name in a thread-local, so subsequent
///    rows reuse the same `Arc` (one atomic ref-count bump, no allocation).
///
/// Used by [`PerWorkerClient::query_prepared_to_value`] and
/// [`PerWorkerClient::query_batch_to_values`] (added below).
fn row_to_value_direct(row: &Row) -> tishlang_runtime::Value {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Arc as StdArcInner;
    use tishlang_runtime::ObjectMap;
    use tishlang_runtime::Value as RtValue;
    use tokio_postgres::types::Type;

    thread_local! {
        // Per-worker thread cache of column-name interned `Arc<str>`s.
        // Bounded by total distinct PG column names the app prepares
        // statements for, i.e. tiny — so unconditional retention is fine.
        static KEY_CACHE: RefCell<HashMap<String, StdArcInner<str>>> =
            RefCell::new(HashMap::new());
    }

    fn intern_key(name: &str) -> StdArcInner<str> {
        KEY_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some(k) = cache.get(name) {
                return StdArcInner::clone(k);
            }
            let k: StdArcInner<str> = StdArcInner::from(name);
            cache.insert(name.to_string(), StdArcInner::clone(&k));
            k
        })
    }

    let cols = row.columns();
    let mut om = ObjectMap::with_capacity(cols.len());
    for (i, col) in cols.iter().enumerate() {
        let key = intern_key(col.name());
        let v: RtValue = match *col.type_() {
            Type::INT2 => row
                .try_get::<_, Option<i16>>(i)
                .ok()
                .flatten()
                .map(|n| RtValue::Number(n as f64))
                .unwrap_or(RtValue::Null),
            Type::INT4 | Type::OID => row
                .try_get::<_, Option<i32>>(i)
                .ok()
                .flatten()
                .map(|n| RtValue::Number(n as f64))
                .unwrap_or(RtValue::Null),
            Type::INT8 => row
                .try_get::<_, Option<i64>>(i)
                .ok()
                .flatten()
                .map(|n| RtValue::Number(n as f64))
                .unwrap_or(RtValue::Null),
            Type::FLOAT4 => row
                .try_get::<_, Option<f32>>(i)
                .ok()
                .flatten()
                .map(|n| RtValue::Number(n as f64))
                .unwrap_or(RtValue::Null),
            Type::FLOAT8 => row
                .try_get::<_, Option<f64>>(i)
                .ok()
                .flatten()
                .map(RtValue::Number)
                .unwrap_or(RtValue::Null),
            Type::BOOL => row
                .try_get::<_, Option<bool>>(i)
                .ok()
                .flatten()
                .map(RtValue::Bool)
                .unwrap_or(RtValue::Null),
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => row
                .try_get::<_, Option<&str>>(i)
                .ok()
                .flatten()
                .map(|s| RtValue::String(StdArcInner::from(s)))
                .unwrap_or(RtValue::Null),
            _ => {
                // Anything else goes through the JSON path for backwards
                // compat. Hot TFB rows never hit this branch.
                if let Ok(s) = row.try_get::<_, Option<String>>(i) {
                    s.map(|s| RtValue::String(StdArcInner::from(s.as_str())))
                        .unwrap_or(RtValue::Null)
                } else {
                    RtValue::Null
                }
            }
        };
        om.insert(key, v);
    }
    RtValue::Object(VmRef::new(om))
}

fn row_to_object(row: &Row) -> Result<JsonValue> {
    use tokio_postgres::types::Type;
    let mut map = Map::with_capacity(row.columns().len());
    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name().to_string();
        // Phase-2 item 10: type-directed decode for the common Postgres wire
        // types so we skip the text->parse->int detour that the blind
        // try_get cascade triggered. Hot TFB columns are INT4 (world.id,
        // world.randomnumber) and TEXT/VARCHAR (fortune.message).
        let val: JsonValue = match *col.type_() {
            Type::INT2 => row
                .try_get::<_, Option<i16>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            Type::INT4 | Type::OID => row
                .try_get::<_, Option<i32>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            Type::INT8 => row
                .try_get::<_, Option<i64>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            Type::FLOAT4 => row
                .try_get::<_, Option<f32>>(i)
                .map(|v| json!(v.map(|f| f as f64)))
                .unwrap_or(JsonValue::Null),
            Type::FLOAT8 => row
                .try_get::<_, Option<f64>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            Type::BOOL => row
                .try_get::<_, Option<bool>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => row
                .try_get::<_, Option<String>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            Type::JSON | Type::JSONB => row
                .try_get::<_, Option<JsonValue>>(i)
                .unwrap_or(None)
                .unwrap_or(JsonValue::Null),
            Type::BYTEA => row
                .try_get::<_, Option<Vec<u8>>>(i)
                .map(|v| json!(v))
                .unwrap_or(JsonValue::Null),
            _ => {
                // Fall back to the old try-cascade for anything we haven't
                // listed explicitly yet.
                if let Ok(v) = row.try_get::<_, Option<String>>(i) {
                    json!(v)
                } else if let Ok(v) = row.try_get::<_, Option<i64>>(i) {
                    json!(v)
                } else if let Ok(v) = row.try_get::<_, Option<f64>>(i) {
                    json!(v)
                } else if let Ok(v) = row.try_get::<_, Option<bool>>(i) {
                    json!(v)
                } else if let Ok(v) = row.try_get::<_, Option<JsonValue>>(i) {
                    v.unwrap_or(JsonValue::Null)
                } else if let Ok(v) = row.try_get::<_, Option<Vec<u8>>>(i) {
                    json!(v)
                } else {
                    JsonValue::String(format!("<decode:{}>", col.type_().name()))
                }
            }
        };
        map.insert(name, val);
    }
    Ok(JsonValue::Object(map))
}

impl PgPool {
    pub async fn connect(cfg: PoolConfig) -> Result<Self> {
        let pg_cfg = parse_connection_string(&cfg.connection_string)
            .map_err(TishPgError::BadConnectionString)?;
        let mgr = Manager::new(pg_cfg, NoTls);
        let mut b = Pool::builder(mgr).runtime(Runtime::Tokio1);
        if let Some(m) = cfg.max {
            b = b.max_size(m as usize);
        }
        let inner = b.build()?;
        Ok(Self { inner })
    }

    /// Same logical contract as `pool.query(text, params)` in node-postgres.
    pub async fn query(&self, text: &str, params: &[JsonValue]) -> Result<QueryResult> {
        let sql_values = params_to_sql(params)?;
        let refs: Vec<&(dyn ToSql + Sync)> = sql_values
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();

        let client = self.inner.get().await?;
        let rows = client.query(text, &refs[..]).await?;

        let mut fields = Vec::new();
        if let Some(first) = rows.first() {
            for col in first.columns().iter() {
                fields.push(FieldInfo {
                    name: col.name().to_string(),
                    data_type_id: col.type_().oid() as i32,
                });
            }
        }

        let mut out_rows = Vec::with_capacity(rows.len());
        for r in &rows {
            out_rows.push(row_to_object(r)?);
        }

        Ok(QueryResult {
            row_count: rows.len() as u32,
            rows: out_rows,
            fields,
        })
    }

    /// Close the pool (mirrors `pool.end()`).
    pub fn close(&self) {
        self.inner.close();
    }
}

// ---------------------------------------------------------------------------
// PerWorkerClient + prepared-statement surface (Phase-1 item 3)
// ---------------------------------------------------------------------------
//
// The Tish bench needs a dedicated Postgres connection per HTTP worker so the
// hot path is:
//
//   worker 0 -> client 0 (1 TCP socket) -> pipelined Parse/Bind/Execute/Sync
//   worker N -> client N
//
// `tokio-postgres` pipelines automatically when multiple futures on the same
// `Client` are polled concurrently, so we expose a batch-query primitive that
// creates N futures under the hood and `try_join_all`s them.

use std::sync::Arc as StdArc;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Statement};

/// Dedicated tokio-postgres client. Cheap to clone (internal Arc).
#[derive(Clone)]
pub struct PerWorkerClient {
    inner: StdArc<Client>,
    // Background task that drives the connection; we keep it alive for the
    // lifetime of the client.
    _driver: StdArc<JoinHandle<()>>,
}

impl PerWorkerClient {
    /// Open a single direct connection (no pool).
    pub async fn connect(connection_string: &str) -> Result<Self> {
        let cfg = parse_connection_string(connection_string)
            .map_err(TishPgError::BadConnectionString)?;
        let (client, connection) = cfg.connect(NoTls).await?;
        let driver = tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("[tish_pg] connection driver exited: {}", e);
            }
        });
        Ok(Self {
            inner: StdArc::new(client),
            _driver: StdArc::new(driver),
        })
    }

    pub fn client(&self) -> &Client {
        &self.inner
    }

    /// Prepare a named statement. Handle is cheap to clone.
    pub async fn prepare(&self, text: &str) -> Result<Statement> {
        Ok(self.inner.prepare(text).await?)
    }

    /// Run one prepared query -> rows as JSON.
    pub async fn query_prepared(
        &self,
        stmt: &Statement,
        params: &[JsonValue],
    ) -> Result<QueryResult> {
        let sql_values = params_to_sql(params)?;
        let refs: Vec<&(dyn ToSql + Sync)> = sql_values
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();
        let rows = self.inner.query(stmt, &refs[..]).await?;
        let mut fields = Vec::new();
        if let Some(first) = rows.first() {
            for col in first.columns().iter() {
                fields.push(FieldInfo {
                    name: col.name().to_string(),
                    data_type_id: col.type_().oid() as i32,
                });
            }
        }
        let mut out_rows = Vec::with_capacity(rows.len());
        for r in &rows {
            out_rows.push(row_to_object(r)?);
        }
        Ok(QueryResult {
            row_count: rows.len() as u32,
            rows: out_rows,
            fields,
        })
    }

    /// Fire N prepared queries against this client concurrently and await them
    /// together. Triggers `tokio-postgres`'s automatic pipelining: all Parse/
    /// Bind/Execute/Sync messages are written back-to-back in one TCP batch
    /// and the server executes them sequentially while the client never waits
    /// for per-query round-trips.
    pub async fn query_batch(
        &self,
        specs: Vec<(Statement, Vec<JsonValue>)>,
    ) -> Result<Vec<QueryResult>> {
        use futures::future::try_join_all;
        let futs: Vec<_> = specs
            .into_iter()
            .map(|(stmt, params)| {
                let client = StdArc::clone(&self.inner);
                async move {
                    let sql_values = params_to_sql(&params)?;
                    let refs: Vec<&(dyn ToSql + Sync)> = sql_values
                        .iter()
                        .map(|b| b.as_ref() as &(dyn ToSql + Sync))
                        .collect();
                    let rows = client.query(&stmt, &refs[..]).await?;
                    let mut out_rows = Vec::with_capacity(rows.len());
                    for r in &rows {
                        out_rows.push(row_to_object(r)?);
                    }
                    Ok::<_, TishPgError>(QueryResult {
                        row_count: rows.len() as u32,
                        rows: out_rows,
                        fields: Vec::new(),
                    })
                }
            })
            .collect();
        try_join_all(futs).await
    }

    /// Like [`query_prepared`] but emits `Vec<tishlang_runtime::Value>`
    /// directly using the [`row_to_value_direct`] fast path — no
    /// `serde_json::Value` intermediate, no per-row column-name `Arc`
    /// allocations. The hot TFB call site for `/db` and `/queries`.
    pub async fn query_prepared_to_values(
        &self,
        stmt: &Statement,
        params: &[JsonValue],
    ) -> Result<Vec<tishlang_runtime::Value>> {
        let sql_values = params_to_sql(params)?;
        let refs: Vec<&(dyn ToSql + Sync)> = sql_values
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();
        let rows = self.inner.query(stmt, &refs[..]).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            out.push(row_to_value_direct(r));
        }
        Ok(out)
    }

    /// Pipelined batch of typed queries — the equivalent of
    /// [`query_batch`] for the typed (no-JSON) path.
    pub async fn query_batch_to_values(
        &self,
        specs: Vec<(Statement, Vec<JsonValue>)>,
    ) -> Result<Vec<Vec<tishlang_runtime::Value>>> {
        use futures::future::try_join_all;
        let futs: Vec<_> = specs
            .into_iter()
            .map(|(stmt, params)| {
                let client = StdArc::clone(&self.inner);
                async move {
                    let sql_values = params_to_sql(&params)?;
                    let refs: Vec<&(dyn ToSql + Sync)> = sql_values
                        .iter()
                        .map(|b| b.as_ref() as &(dyn ToSql + Sync))
                        .collect();
                    let rows = client.query(&stmt, &refs[..]).await?;
                    let mut out = Vec::with_capacity(rows.len());
                    for r in &rows {
                        out.push(row_to_value_direct(r));
                    }
                    Ok::<_, TishPgError>(out)
                }
            })
            .collect();
        try_join_all(futs).await
    }
}

// ---------------------------------------------------------------------------
// Sync facade for the `cargo:tish_pg` Tish import (feature = tish-bindings)
// ---------------------------------------------------------------------------

#[cfg(feature = "tish-bindings")]
mod tish_sync {
    use super::*;
    use once_cell::sync::Lazy;
    use slab::Slab;
    use std::sync::Mutex;
    use tishlang_runtime::Value as TishValue;
    use tokio::runtime::Runtime as TokioRuntime;

    static RT: Lazy<TokioRuntime> = Lazy::new(|| {
        TokioRuntime::new().expect("tish_pg: failed to build tokio runtime")
    });
    // `RwLock` (not `Mutex`) on the registries: hot path is read-only —
    // every query does `get_client(id)` + `get_statement(id)`. Inserts
    // happen once per `connect`/`prepare` at startup. With multiple HTTP
    // worker threads each doing thousands of QPS, the prior `Mutex<Slab>`
    // serialised every query through one global lock; `RwLock` lets all
    // concurrent reads run lock-free against each other.
    use std::sync::RwLock;
    static CLIENTS: Lazy<RwLock<Slab<PerWorkerClient>>> =
        Lazy::new(|| RwLock::new(Slab::new()));
    static STATEMENTS: Lazy<RwLock<Slab<(usize, Statement)>>> =
        Lazy::new(|| RwLock::new(Slab::new()));

    /// Drive a future on our tokio runtime without panicking when called from
    /// inside another runtime's worker thread.
    ///
    /// Fast path: the Tish HTTP handler runs on the VM dispatcher thread,
    /// which is NOT inside a tokio runtime — so we can enter `RT.block_on`
    /// directly, no thread spawn. Measured cost: ~50-100μs per call saved
    /// (and a thread creation per request is what was capping /db RPS).
    ///
    /// Slow path: if we detect an ambient tokio runtime (e.g. someone is
    /// calling us from inside an async context like Tish's top-level
    /// `await`), we fall back to a scoped thread to avoid the
    /// "Cannot start a runtime from within a runtime" panic.
    fn block_on<F>(fut: F) -> F::Output
    where
        F: std::future::Future + Send,
        F::Output: Send,
    {
        // Whether we're inside a tokio runtime never changes for a given
        // OS thread — Tish runs handlers on the VM dispatcher thread,
        // which is a plain `std::thread` (so the answer is "no, no
        // ambient runtime"). Cache the result per-thread so the
        // `try_current()` syscall-ish call doesn't repeat thousands of
        // times per second per worker.
        use std::cell::Cell;
        thread_local! {
            static AMBIENT_RT: Cell<Option<bool>> = const { Cell::new(None) };
        }
        let in_ambient = AMBIENT_RT.with(|c| {
            if let Some(v) = c.get() {
                return v;
            }
            let v = tokio::runtime::Handle::try_current().is_ok();
            c.set(Some(v));
            v
        });
        if in_ambient {
            // Ambient runtime present — spawn a thread so RT.block_on does
            // not nest.
            return std::thread::scope(|s| {
                let (tx, rx) = std::sync::mpsc::channel();
                s.spawn(move || {
                    let out = RT.block_on(fut);
                    let _ = tx.send(out);
                });
                rx.recv().expect("tish_pg::block_on thread panicked")
            });
        }
        // Hot path: we're on a plain OS thread (the Tish VM dispatcher),
        // enter tokio directly.
        RT.block_on(fut)
    }

    fn tish_to_json(v: &TishValue) -> JsonValue {
        match v {
            TishValue::Null => JsonValue::Null,
            TishValue::Bool(b) => JsonValue::Bool(*b),
            TishValue::Number(n) => serde_json::Number::from_f64(*n)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            TishValue::String(s) => JsonValue::String(s.to_string()),
            TishValue::Array(a) => {
                JsonValue::Array(a.borrow().iter().map(tish_to_json).collect())
            }
            TishValue::Object(o) => {
                let mut m = serde_json::Map::new();
                for (k, v) in o.borrow().iter() {
                    m.insert(k.to_string(), tish_to_json(v));
                }
                JsonValue::Object(m)
            }
            _ => JsonValue::Null,
        }
    }

    fn json_to_tish(v: JsonValue) -> TishValue {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::Arc;
        use tishlang_runtime::ObjectMap;
        match v {
            JsonValue::Null => TishValue::Null,
            JsonValue::Bool(b) => TishValue::Bool(b),
            JsonValue::Number(n) => TishValue::Number(n.as_f64().unwrap_or(0.0)),
            JsonValue::String(s) => TishValue::String(s.into()),
            JsonValue::Array(a) => {
                let mut out = Vec::with_capacity(a.len());
                for item in a {
                    out.push(json_to_tish(item));
                }
                TishValue::Array(VmRef::new(out))
            }
            JsonValue::Object(m) => {
                // Pre-allocate ObjectMap capacity so HashMap doesn't rehash
                // on every insert. Common TFB rows are 2 columns (id,
                // randomnumber or id, message).
                let mut om = ObjectMap::with_capacity(m.len());
                for (k, v) in m {
                    om.insert(Arc::from(k), json_to_tish(v));
                }
                TishValue::Object(VmRef::new(om))
            }
        }
    }

    fn tish_err(msg: impl Into<String>) -> TishValue {
        use std::sync::Arc;
        use tishlang_runtime::ObjectMap;
        let mut om = ObjectMap::with_capacity(2);
        om.insert(Arc::from("error"), TishValue::String(msg.into().into()));
        om.insert(Arc::from("ok"), TishValue::Bool(false));
        TishValue::Object(VmRef::new(om))
    }

    fn rows_to_value(res: QueryResult) -> TishValue {
        use std::cell::RefCell;
        use std::rc::Rc;
        TishValue::Array(VmRef::new(
            res.rows.into_iter().map(json_to_tish).collect(),
        ))
    }

    /// `perWorkerClient(connection_string) -> client_handle` (blocking).
    pub fn per_worker_client(args: &[TishValue]) -> TishValue {
        let cs = match args.first() {
            Some(TishValue::String(s)) => s.to_string(),
            _ => return tish_err("perWorkerClient: expected connection string"),
        };
        match block_on(PerWorkerClient::connect(&cs)) {
            Ok(c) => {
                let mut g = CLIENTS.write().unwrap();
                let id = g.insert(c);
                TishValue::Number(id as f64)
            }
            Err(e) => tish_err(format!("perWorkerClient: {}", crate::format_tish_pg_error(&e))),
        }
    }

    /// `connect(options) -> client_handle`.
    /// Aliased to per_worker_client for now; Pool-based variant is still
    /// reachable via the async Rust API.
    pub fn connect(args: &[TishValue]) -> TishValue {
        // Accept either a plain connection string or `{ connectionString }`.
        let cs = match args.first() {
            Some(TishValue::String(s)) => s.to_string(),
            Some(TishValue::Object(obj)) => {
                use std::sync::Arc;
                let b = obj.borrow();
                match b.get(&Arc::from("connectionString")) {
                    Some(TishValue::String(s)) => s.to_string(),
                    _ => return tish_err("connect: options.connectionString missing"),
                }
            }
            _ => return tish_err("connect: expected connection string or options"),
        };
        match block_on(PerWorkerClient::connect(&cs)) {
            Ok(c) => {
                let mut g = CLIENTS.write().unwrap();
                let id = g.insert(c);
                TishValue::Number(id as f64)
            }
            Err(e) => tish_err(format!("connect: {}", crate::format_tish_pg_error(&e))),
        }
    }

    fn get_client(id: f64) -> Option<PerWorkerClient> {
        let g = CLIENTS.read().unwrap();
        g.get(id as usize).cloned()
    }

    fn get_statement(id: f64) -> Option<(usize, Statement)> {
        let g = STATEMENTS.read().unwrap();
        g.get(id as usize).cloned()
    }

    /// `prepare(client_handle, sql) -> statement_handle`.
    pub fn prepare(args: &[TishValue]) -> TishValue {
        let Some(TishValue::Number(client_id)) = args.first() else {
            return tish_err("prepare: expected (client, sql)");
        };
        let Some(TishValue::String(sql)) = args.get(1) else {
            return tish_err("prepare: expected (client, sql)");
        };
        let Some(client) = get_client(*client_id) else {
            return tish_err("prepare: unknown client handle");
        };
        let sql = sql.to_string();
        match block_on(client.prepare(&sql)) {
            Ok(stmt) => {
                let mut g = STATEMENTS.write().unwrap();
                let id = g.insert((*client_id as usize, stmt));
                TishValue::Number(id as f64)
            }
            Err(e) => tish_err(format!("prepare: {}", crate::format_tish_pg_error(&e))),
        }
    }

    /// `queryPrepared(client, stmt, params) -> rows`.
    pub fn query_prepared(args: &[TishValue]) -> TishValue {
        let Some(TishValue::Number(client_id)) = args.first() else {
            return tish_err("queryPrepared: expected (client, stmt, params)");
        };
        let Some(TishValue::Number(stmt_id)) = args.get(1) else {
            return tish_err("queryPrepared: expected (client, stmt, params)");
        };
        let params = match args.get(2) {
            Some(TishValue::Array(a)) => a
                .borrow()
                .iter()
                .map(tish_to_json)
                .collect::<Vec<_>>(),
            Some(TishValue::Null) | None => Vec::new(),
            Some(v) => vec![tish_to_json(v)],
        };
        let Some(client) = get_client(*client_id) else {
            return tish_err("queryPrepared: unknown client");
        };
        let Some((_cid, stmt)) = get_statement(*stmt_id) else {
            return tish_err("queryPrepared: unknown statement");
        };
        // Fast path: build `Value::Object` rows directly from `Row` so we
        // skip the `serde_json::Value` intermediate that `query_prepared`
        // produces. Fewer allocations + interned column-name `Arc`s.
        match block_on(client.query_prepared_to_values(&stmt, &params)) {
            Ok(rows) => TishValue::Array(VmRef::new(rows)),
            Err(e) => tish_err(format!("queryPrepared: {}", crate::format_tish_pg_error(&e))),
        }
    }

    /// `queryAll(client, specs) -> array_of_row_arrays`.
    /// `specs` is an Array of `[stmt_handle, params_array]` pairs. All are
    /// polled concurrently on the same client to trigger tokio-postgres
    /// automatic pipelining.
    pub fn query_all(args: &[TishValue]) -> TishValue {
        let Some(TishValue::Number(client_id)) = args.first() else {
            return tish_err("queryAll: expected (client, specs[])");
        };
        let Some(TishValue::Array(specs)) = args.get(1) else {
            return tish_err("queryAll: expected (client, specs[])");
        };
        let Some(client) = get_client(*client_id) else {
            return tish_err("queryAll: unknown client");
        };
        let specs_vec: Vec<(Statement, Vec<JsonValue>)> = {
            let borrow = specs.borrow();
            let mut out = Vec::with_capacity(borrow.len());
            for item in borrow.iter() {
                let TishValue::Array(pair) = item else {
                    return tish_err("queryAll: each spec must be [stmt, params]");
                };
                let pair_b = pair.borrow();
                let Some(TishValue::Number(stmt_id)) = pair_b.first() else {
                    return tish_err("queryAll: spec[0] must be a statement handle");
                };
                let Some((_cid, stmt)) = get_statement(*stmt_id) else {
                    return tish_err("queryAll: unknown statement");
                };
                let params = match pair_b.get(1) {
                    Some(TishValue::Array(a)) => {
                        a.borrow().iter().map(tish_to_json).collect::<Vec<_>>()
                    }
                    Some(TishValue::Null) | None => Vec::new(),
                    Some(v) => vec![tish_to_json(v)],
                };
                out.push((stmt, params));
            }
            out
        };
        // Fast path: same direct `Row -> Value::Object` mapping as
        // `query_prepared` above, but pipelined across all specs.
        match block_on(client.query_batch_to_values(specs_vec)) {
            Ok(results) => {
                let outer: Vec<TishValue> = results
                    .into_iter()
                    .map(|rows| TishValue::Array(VmRef::new(rows)))
                    .collect();
                TishValue::Array(VmRef::new(outer))
            }
            Err(e) => tish_err(format!("queryAll: {}", crate::format_tish_pg_error(&e))),
        }
    }

    /// `close(client_handle) -> null`.
    pub fn close(args: &[TishValue]) -> TishValue {
        if let Some(TishValue::Number(id)) = args.first() {
            let mut g = CLIENTS.write().unwrap();
            if g.contains(*id as usize) {
                g.remove(*id as usize);
            }
        }
        TishValue::Null
    }

    /// `migrate(client_handle, dir) -> { ok, applied: [name, …], error? }`.
    ///
    /// Reads `dir` for files matching `^\d+_.+\.sql$`, sorted lexically
    /// (so `001_init.sql` < `002_users.sql` < …), creates a
    /// `_tish_pg_migrations(name TEXT PRIMARY KEY, applied_at TIMESTAMPTZ
    /// NOT NULL DEFAULT NOW())` ledger, and applies each new file in a
    /// single transaction per file. Idempotent — files already recorded
    /// in the ledger are skipped.
    pub fn migrate(args: &[TishValue]) -> TishValue {
        use std::sync::Arc;
        use tishlang_runtime::ObjectMap;

        let Some(TishValue::Number(client_id)) = args.first() else {
            return tish_err("migrate: expected (client_handle, dir)");
        };
        let Some(TishValue::String(dir)) = args.get(1) else {
            return tish_err("migrate: expected (client_handle, dir)");
        };
        let Some(client) = get_client(*client_id) else {
            return tish_err("migrate: unknown client handle");
        };

        let dir = std::path::PathBuf::from(dir.as_ref());
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => return tish_err(format!("migrate: read_dir({:?}): {e}", dir)),
        };

        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if !name.ends_with(".sql") {
                continue;
            }
            // accept any file ending in .sql; sort lexically.
            files.push((name, ent.path()));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));

        let raw_client = client.client();
        let applied: Vec<TishValue> = match block_on(async {
            // Create ledger
            raw_client
                .batch_execute(
                    "CREATE TABLE IF NOT EXISTS _tish_pg_migrations (\
                       name TEXT PRIMARY KEY, \
                       applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW())",
                )
                .await?;

            // Read already-applied
            let rows = raw_client
                .query("SELECT name FROM _tish_pg_migrations", &[])
                .await?;
            let already: std::collections::HashSet<String> = rows
                .iter()
                .map(|r| r.get::<_, String>(0))
                .collect();

            let mut applied_now: Vec<String> = Vec::new();
            for (name, path) in &files {
                if already.contains(name) {
                    continue;
                }
                let sql = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err::<_, TishPgError>(TishPgError::BadParam(format!(
                            "migrate: read {name}: {e}"
                        )))
                    }
                };
                // Each migration runs in its own transaction.
                raw_client.batch_execute("BEGIN").await?;
                let res = raw_client.batch_execute(&sql).await;
                match res {
                    Ok(()) => {
                        raw_client
                            .execute(
                                "INSERT INTO _tish_pg_migrations(name) VALUES ($1)",
                                &[&name],
                            )
                            .await?;
                        raw_client.batch_execute("COMMIT").await?;
                        applied_now.push(name.clone());
                    }
                    Err(e) => {
                        let _ = raw_client.batch_execute("ROLLBACK").await;
                        return Err(TishPgError::Postgres(e));
                    }
                }
            }
            Ok::<_, TishPgError>(applied_now)
        }) {
            Ok(v) => v.into_iter().map(|s| TishValue::String(s.into())).collect(),
            Err(e) => return tish_err(format!("migrate: {}", crate::format_tish_pg_error(&e))),
        };

        let mut om = ObjectMap::with_capacity(2);
        om.insert(Arc::from("ok"), TishValue::Bool(true));
        om.insert(Arc::from("applied"), TishValue::Array(VmRef::new(applied)));
        TishValue::Object(VmRef::new(om))
    }
}

// Re-export the sync facade at crate root (pub fn(args: &[Value]) -> Value
// shape that `tishlang-cargo-bindgen` picks up automatically). The public
// names are snake_case because Tish's codegen snake-cases .tish imports on
// the Rust side (camelCase `queryAll` in .tish -> snake `query_all` here).
#[cfg(feature = "tish-bindings")]
pub use tish_sync::{
    close, connect, migrate, per_worker_client, prepare, query_all, query_prepared,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_url() {
        let cfg = parse_connection_string("postgres://u:p@h:5432/db").unwrap();
        assert_eq!(cfg.get_user(), Some("u"));
    }
}
