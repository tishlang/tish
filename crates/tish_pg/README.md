# tishlang_pg

**PostgreSQL for Tish** — a Rust library (`rlib`) that **Tish-compiled programs** use: Tish compiles to native Rust, and this crate is a normal Rust dependency of that output, so **authors still write `.tish`**.

## What this is **not**

- **No Node.js** / no N-API in this crate
- No JavaScript runtime in the driver itself

## What this **is**

- **`tokio-postgres`** + **`deadpool-postgres`** for the wire protocol and pooling.
- A **Rust API** (`PgPool`, `PoolConfig`, `QueryResult`) plus Tish-facing bindings (`cargo:tish_pg`) used when the CLI is built with the **`pg`** feature.

## From `.tish` (npm package)

Install the scoped package and import the same surface as the standalone `tish-pg` repo used to ship:

```tish
import { connect, queryPrepared, prepare, close } from '@tishlang/pg'
```

Requires **`tish run` / `tish build` with the `pg` feature** (included in the default `full` feature set).

## Building in this workspace

From the `tish` repo root:

```bash
cargo build -p tishlang_pg
cargo test -p tishlang_pg
```

See the workspace [`justfile`](../../justfile) for common tasks.

## License

Same as the parent workspace (`LICENSE` at repo root).
