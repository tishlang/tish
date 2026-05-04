use std::fmt::Write as _;
use std::error::Error as StdError;

use deadpool::managed::PoolError;
use thiserror::Error;

/// `tokio_postgres::Error`’s [`Display`] is often just `"db error"` for server
/// faults; the real text lives on [`tokio_postgres::Error::as_db_error`].
pub fn format_pg_error(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        let mut s = format!("{} [{}]", db.message(), db.code().code());
        if let Some(d) = db.detail() {
            let _ = write!(s, " ({})", d);
        }
        if let Some(h) = db.hint() {
            let _ = write!(s, " hint: {}", h);
        }
        return s;
    }
    let mut out = e.to_string();
    if let Some(src) = e.source() {
        let _ = write!(out, " ({})", src);
    }
    out
}

pub fn format_tish_pg_error(e: &TishPgError) -> String {
    match e {
        TishPgError::Postgres(pg) => format_pg_error(pg),
        TishPgError::Pool(pe) => match pe {
            PoolError::Backend(pg) => format_pg_error(pg),
            _ => pe.to_string(),
        },
        _ => e.to_string(),
    }
}

#[derive(Debug, Error)]
pub enum TishPgError {
    #[error("invalid connection string: {0}")]
    BadConnectionString(String),
    #[error("invalid query parameter: {0}")]
    BadParam(String),
    #[error("postgres: {0}")]
    Postgres(#[from] tokio_postgres::Error),
    #[error("pool: {0}")]
    Pool(#[from] deadpool::managed::PoolError<tokio_postgres::Error>),
    #[error("build pool: {0}")]
    Build(#[from] deadpool_postgres::BuildError),
}

pub type Result<T> = std::result::Result<T, TishPgError>;
