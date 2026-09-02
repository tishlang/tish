//! Prepared-statement bookkeeping for the sync `cargo:tish_pg` facade.
//!
//! This is the `STATEMENTS` slab from `tish_sync`, extracted as a pure data
//! structure so its lifecycle semantics are unit-testable without a live
//! Postgres (`tokio_postgres::Statement` cannot be constructed outside a real
//! connection, so the registry is generic over the statement type `S`).
//!
//! Lifecycle contract (tishlang/tish#712):
//!
//! * `get` validates the stored client id — a handle prepared on one client
//!   never resolves against another. `close()` frees CLIENTS slab indices
//!   for reuse, so without this check a stale handle would be replayed
//!   against whichever new connection reused the index.
//! * `insert` interns on `(client_id, sql)` so repeated prepares of
//!   identical SQL reuse one entry — bounding the table by distinct SQL
//!   text (program text) instead of call count.
//! * `sweep_client` drops every entry belonging to a closed client,
//!   including its intern-map slice, so a later client that reuses the slab
//!   index starts clean.
//! * `remove` (the `unprepare` export) frees a single entry.
//!
//! Removal hands the statement back to the caller: dropping a
//! `tokio_postgres::Statement` emits the server-side `DEALLOCATE` (`Close`
//! message) while its connection is still alive, so "drop what sweep/remove
//! return" is the deallocation path.

use slab::Slab;
use std::collections::HashMap;

pub(crate) struct StatementRegistry<S> {
    /// stmt_id -> (client_id, sql, statement). The SQL text rides along so
    /// `remove`/`sweep_client` can purge the matching intern entry; `get`
    /// clones only the statement, never the SQL, keeping the per-query hot
    /// path allocation-free.
    slab: Slab<(usize, String, S)>,
    /// client_id -> sql -> stmt_id. Nested (rather than keyed on a
    /// `(usize, String)` tuple) so `lookup` borrows the SQL without
    /// allocating and `sweep_client` drops a client's slice in O(1).
    intern: HashMap<usize, HashMap<String, usize>>,
}

impl<S: Clone> StatementRegistry<S> {
    pub(crate) fn new() -> Self {
        Self {
            slab: Slab::new(),
            intern: HashMap::new(),
        }
    }

    /// Register a statement prepared on `client_id` for `sql`.
    ///
    /// Idempotent per `(client_id, sql)`: if an entry is already interned
    /// its existing handle is returned and `stmt` is discarded — for a
    /// `tokio_postgres::Statement` the drop emits the server-side
    /// `DEALLOCATE`, so losing a race to a concurrent identical `prepare`
    /// leaves nothing pinned on either side.
    pub(crate) fn insert(&mut self, client_id: usize, sql: &str, stmt: S) -> usize {
        if let Some(id) = self.lookup(client_id, sql) {
            return id;
        }
        let id = self.slab.insert((client_id, sql.to_string(), stmt));
        self.intern
            .entry(client_id)
            .or_default()
            .insert(sql.to_string(), id);
        id
    }

    /// Resolve a statement handle on behalf of `client_id`.
    ///
    /// A mismatch between the stored and presented client id is a MISS:
    /// after `close()` recycles a CLIENTS slab index, a stale statement
    /// handle must not replay against the new connection.
    pub(crate) fn get(&self, stmt_id: usize, client_id: usize) -> Option<S> {
        match self.slab.get(stmt_id) {
            Some((cid, _sql, stmt)) if *cid == client_id => Some(stmt.clone()),
            _ => None,
        }
    }

    /// Free one statement entry (the `unprepare` export), returning the
    /// statement so the caller can drop it (which sends `DEALLOCATE`).
    pub(crate) fn remove(&mut self, stmt_id: usize) -> Option<S> {
        if !self.slab.contains(stmt_id) {
            return None;
        }
        let (cid, sql, stmt) = self.slab.remove(stmt_id);
        if let Some(per_client) = self.intern.get_mut(&cid) {
            per_client.remove(&sql);
            if per_client.is_empty() {
                self.intern.remove(&cid);
            }
        }
        Some(stmt)
    }

    /// Drop every entry belonging to `client_id` (the `close()` sweep);
    /// returns how many were removed. The dropped statements emit their
    /// `DEALLOCATE`s if the connection still lives; purging the intern
    /// slice keeps a later client that reuses this index from inheriting
    /// stale handles.
    pub(crate) fn sweep_client(&mut self, client_id: usize) -> usize {
        let before = self.slab.len();
        self.slab.retain(|_, (cid, _sql, _stmt)| *cid != client_id);
        self.intern.remove(&client_id);
        before - self.slab.len()
    }

    /// Find the interned handle for `(client_id, sql)`, if one exists.
    pub(crate) fn lookup(&self, client_id: usize, sql: &str) -> Option<usize> {
        self.intern.get(&client_id)?.get(sql).copied()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slab.len()
    }
}

#[cfg(test)]
mod tests {
    use super::StatementRegistry;

    fn s(x: &str) -> String {
        x.to_string()
    }

    #[test]
    fn get_with_matching_client_id_hits() {
        let mut r = StatementRegistry::new();
        let id = r.insert(3, "SELECT 1", s("stmt-a"));
        assert_eq!(r.get(id, 3), Some(s("stmt-a")));
    }

    #[test]
    fn get_with_mismatched_client_id_misses() {
        // The stale-replay guard: a handle prepared on client 3 must never
        // resolve for client 4. After close() frees a CLIENTS slab index for
        // reuse, an old statement handle would otherwise be replayed against
        // whichever new connection reused the index.
        let mut r = StatementRegistry::new();
        let id = r.insert(3, "SELECT 1", s("stmt-a"));
        assert_eq!(
            r.get(id, 4),
            None,
            "statement prepared on client 3 must MISS when resolved for client 4"
        );
    }

    #[test]
    fn close_sweep_removes_only_that_clients_statements() {
        let mut r = StatementRegistry::new();
        let a = r.insert(1, "SELECT 1", s("c1-a"));
        let b = r.insert(1, "SELECT 2", s("c1-b"));
        let c = r.insert(2, "SELECT 1", s("c2-a"));
        let swept = r.sweep_client(1);
        assert_eq!(swept, 2, "both of client 1's statements must be swept");
        assert_eq!(r.get(a, 1), None);
        assert_eq!(r.get(b, 1), None);
        assert_eq!(r.get(c, 2), Some(s("c2-a")), "client 2 must be untouched");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn unprepare_removes_entry() {
        let mut r = StatementRegistry::new();
        let id = r.insert(1, "SELECT 1", s("stmt-a"));
        assert_eq!(
            r.remove(id),
            Some(s("stmt-a")),
            "remove must yield the statement so its Drop can DEALLOCATE"
        );
        assert_eq!(r.get(id, 1), None);
        assert_eq!(r.remove(id), None, "double-unprepare is a miss");
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn reprepare_same_sql_reuses_interned_entry() {
        // prepare() must be idempotent per (client, sql): the misuse pattern
        // of preparing inside a request handler must not grow the table.
        let mut r = StatementRegistry::new();
        let first = r.insert(1, "SELECT 1", s("stmt-a"));
        let second = r.insert(1, "SELECT 1", s("stmt-b"));
        assert_eq!(
            second, first,
            "identical (client, sql) must reuse the entry"
        );
        assert_eq!(r.len(), 1);
        // The original statement wins; the racing duplicate is dropped by the
        // caller (its Drop emits the server-side DEALLOCATE).
        assert_eq!(r.get(first, 1), Some(s("stmt-a")));
    }

    #[test]
    fn distinct_sql_or_client_gets_distinct_entries() {
        // The intern bound is (client, distinct SQL) — different SQL on the
        // same client, and the same SQL on different clients, each get their
        // own entry.
        let mut r = StatementRegistry::new();
        let a = r.insert(1, "SELECT 1", s("c1-sel1"));
        let b = r.insert(1, "SELECT 2", s("c1-sel2"));
        let c = r.insert(2, "SELECT 1", s("c2-sel1"));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(r.len(), 3);
        assert_eq!(r.get(a, 1), Some(s("c1-sel1")));
        assert_eq!(r.get(b, 1), Some(s("c1-sel2")));
        assert_eq!(r.get(c, 2), Some(s("c2-sel1")));
    }

    #[test]
    fn lookup_finds_interned_entry() {
        let mut r = StatementRegistry::new();
        assert_eq!(r.lookup(1, "SELECT 1"), None);
        let id = r.insert(1, "SELECT 1", s("stmt-a"));
        assert_eq!(r.lookup(1, "SELECT 1"), Some(id));
        assert_eq!(r.lookup(2, "SELECT 1"), None, "interning is per client");
        assert_eq!(r.lookup(1, "SELECT 2"), None, "interning is per SQL text");
    }

    #[test]
    fn sweep_purges_intern_state_for_reused_client_index() {
        // After close(client 0), a NEW client that reuses slab index 0 must
        // not inherit the old client's statements or interned handles.
        let mut r = StatementRegistry::new();
        r.insert(0, "SELECT 1", s("old-stmt"));
        r.sweep_client(0);
        assert_eq!(
            r.lookup(0, "SELECT 1"),
            None,
            "a reused client index must re-prepare, not inherit the old client's intern entry"
        );
        assert_eq!(r.len(), 0, "the closed client's statements must be gone");
        let new_id = r.insert(0, "SELECT 1", s("new-stmt"));
        assert_eq!(r.get(new_id, 0), Some(s("new-stmt")));
    }

    #[test]
    fn unprepare_then_reprepare_gets_fresh_entry() {
        // remove() must purge the intern entry too, or a later prepare of
        // the same SQL would return a dangling handle.
        let mut r = StatementRegistry::new();
        let id = r.insert(1, "SELECT 1", s("stmt-a"));
        r.remove(id);
        assert_eq!(r.lookup(1, "SELECT 1"), None);
        let id2 = r.insert(1, "SELECT 1", s("stmt-b"));
        assert_eq!(r.get(id2, 1), Some(s("stmt-b")));
    }
}
