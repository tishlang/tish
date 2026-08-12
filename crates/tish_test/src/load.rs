//! Load a test file into the suite registry (requires `runner` feature).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::coverage::{self, hit_global_name, hit_value};
use crate::instrument::instrument_program;
use crate::module::{assert_module, test_module};
use crate::registry::{begin_collect, end_collect, take_collect_errors, SuiteNode};

/// A loaded file plus any errors thrown while collecting its suites.
pub struct LoadedSuite {
    pub suite: SuiteNode,
    /// Throws that escaped a `describe` body. These silently truncate registration, so the
    /// runner reports them as failures instead of letting the file look green.
    pub collect_errors: Vec<String>,
}

/// Resolve, merge, optimize, and execute `path` so `describe`/`test` callbacks register
/// into the collect-then-run suite tree. Suppresses printing the program's last value.
///
/// `preload` modules run first **on the same VM instance**, so anything they define or
/// register (globals, `beforeEach` hooks) is visible to the test file. `features` restricts VM
/// capabilities exactly as `tish run --feature` does; an empty list means "everything linked
/// into this binary".
pub fn load_and_collect_tests(
    path: &Path,
    preload: &[PathBuf],
    backend: &str,
    no_optimize: bool,
    features: &[String],
) -> Result<LoadedSuite, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve {}: {}", path.display(), e))?;

    begin_collect(&path.to_string_lossy());

    let mut programs = Vec::with_capacity(preload.len() + 1);
    for pre in preload {
        let pre = pre
            .canonicalize()
            .map_err(|e| format!("Cannot resolve preload {}: {}", pre.display(), e))?;
        let program = compile_program(&pre, no_optimize)
            .map_err(|e| format!("Error loading preload {}: {}", pre.display(), e))?;
        programs.push((pre, program));
    }
    programs.push((path.clone(), compile_program(&path, no_optimize)?));

    let result = match backend {
        "vm" => run_vm(&programs, features),
        other => {
            return Err(format!(
                "unsupported --backend `{other}` for tish test (use `vm`)"
            ));
        }
    };

    let suite = end_collect();
    let collect_errors = take_collect_errors();
    result?;
    Ok(LoadedSuite {
        suite,
        collect_errors,
    })
}

fn compile_program(path: &Path, no_optimize: bool) -> Result<tishlang_ast::Program, String> {
    let project_root = path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });

    let mut modules = tishlang_compile::resolve_project(path, project_root)?;
    tishlang_compile::detect_cycles(&modules)?;

    // Instrument each module's AST in this crate so vm / interp / future backends share hits.
    // A module reached from more than one test file must only be instrumented once, or its
    // statements would be double-counted (and re-wrapped) on the second load.
    if coverage::is_enabled() {
        for m in &mut modules {
            if coverage::mark_instrumented(&m.path) {
                instrument_program(&mut m.program, &m.path);
            }
        }
    }

    let prog = tishlang_compile::merge_modules(modules)?.program;
    Ok(if no_optimize {
        prog
    } else {
        tishlang_opt::optimize(&prog)
    })
}

/// VM capability set, matching `tish run`: empty `--feature` means every capability linked
/// into this binary; otherwise exactly the requested set (`full` expands).
fn vm_capabilities(features: &[String]) -> HashSet<String> {
    if features.is_empty() {
        return tishlang_vm::all_compiled_capabilities();
    }
    let mut out = HashSet::new();
    for s in features {
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "full" {
                for name in ["http", "timers", "fs", "process", "regex", "ws", "tty"] {
                    out.insert(name.to_string());
                }
            } else {
                out.insert(part.to_string());
            }
        }
    }
    out
}

fn run_vm(
    programs: &[(PathBuf, tishlang_ast::Program)],
    features: &[String],
) -> Result<(), String> {
    let mut vm = tishlang_vm::Vm::with_capabilities(vm_capabilities(features));
    // `node:assert` / `assert/strict` are normalized to `tish:assert` during resolution, so
    // only the canonical specs need registering.
    vm.register_native_module("tish:test", test_module());
    vm.register_native_module("tish:assert", assert_module());
    if coverage::is_enabled() {
        vm.set_global(Arc::from(hit_global_name()), hit_value());
    }

    for (path, program) in programs {
        let source_arc: Arc<str> = Arc::from(path.to_string_lossy().as_ref());
        let mut chunk = tishlang_bytecode::compile_with_source(program, Some(source_arc.clone()))
            .map_err(|e| e.to_string())?;
        chunk.source = Some(source_arc);

        vm.run_with_options(&chunk, false)
            .map_err(|e| format!("Error loading {}: {}", path.display(), e))?;
        if tishlang_core::has_pending_throw() {
            let err = tishlang_core::take_pending_throw().unwrap_or(tishlang_core::Value::Null);
            return Err(format!(
                "Error loading {}: {}",
                path.display(),
                err.to_display_string()
            ));
        }
    }
    Ok(())
}
