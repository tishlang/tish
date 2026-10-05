//! `tish:ffi` `loadModule` on the bytecode VM. Loading real modules is covered by
//! `crates/tish_ffi/tests/loader_v2.rs`; this pins the builtin wiring and its error contract.
#![cfg(feature = "ffi")]

use tishlang_bytecode::compile;
use tishlang_vm::run;

fn run_src(src: &str) {
    let cwd = std::env::current_dir().expect("cwd");
    let modules = tishlang_compile::resolve_project_from_stdin(src, &cwd).expect("resolve");
    let program = tishlang_compile::merge_modules(modules).expect("merge").program;
    let chunk = compile(&program).expect("compile");
    run(&chunk).expect("run");
}

#[test]
fn load_failures_throw_catchable_type_errors() {
    run_src(
        r#"
import { loadModule } from "tish:ffi"
if (typeof loadModule !== "function") { throw "loadModule is not wired" }

let missing = ""
try { loadModule("/nonexistent/nimble-test.lib") } catch (e) { missing = e.name + " " + e.message }
if (missing.indexOf("TypeError loadModule:") !== 0) { throw "missing library: " + missing }

let badArg = ""
try { loadModule(42) } catch (e) { badArg = e.name }
if (badArg !== "TypeError") { throw "non-string path: " + badArg }
"#,
    );
}
