//! Native `tish test` runner, Bun/Jest-shaped `tish:test`, and Node-mirrored `tish:assert`.

pub mod assert;
pub mod config;
pub mod coverage;
pub mod deep_equal;
pub mod discovery;
pub mod expect;
pub mod instrument;
pub mod isolation;
pub mod mocks;
pub mod module;
pub mod registry;
pub mod report;
pub mod runner;
pub mod snapshots;

#[cfg(feature = "runner")]
pub mod load;

pub use module::{assert_module, test_module};
pub use runner::{run_tests, run_tests_watch, TestOptions, TestRunResult};
