//! Load a test file into the suite registry (requires `runner` feature).

use std::path::Path;
use std::sync::Arc;

use tishlang_core::ObjectMap;

use crate::coverage::{self, hit_global_name, hit_value};
use crate::instrument::instrument_program;
use crate::module::{assert_module, test_module};
use crate::registry::{begin_collect, end_collect, SuiteNode};

/// Resolve, merge, optimize, and execute `path` so `describe`/`test` callbacks register
/// into the collect-then-run suite tree. Suppresses printing the program's last value.
pub fn load_and_collect_tests(
    path: &Path,
    backend: &str,
    no_optimize: bool,
    _features: &[String],
) -> Result<SuiteNode, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve {}: {}", path.display(), e))?;

    begin_collect(&path.to_string_lossy());

    let project_root = path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });

    let mut modules = tishlang_compile::resolve_project(&path, project_root)?;
    tishlang_compile::detect_cycles(&modules)?;

    // Instrument each module's AST in this crate so vm / interp / future backends share hits.
    if coverage::is_enabled() {
        for m in &mut modules {
            instrument_program(&mut m.program, &m.path);
        }
    }

    let prog = tishlang_compile::merge_modules(modules)?.program;
    let program = if no_optimize {
        prog
    } else {
        tishlang_opt::optimize(&prog)
    };

    let result = match backend {
        "interp" => run_interp(&program, &path),
        "vm" => run_vm(&program, &path),
        other => {
            return Err(format!(
                "unsupported --backend `{other}` for tish test (use `vm` or `interp`; native is not implemented yet)"
            ));
        }
    };

    let suite = end_collect();
    result?;
    Ok(suite)
}

fn test_exports_map() -> ObjectMap {
    test_module()
}

fn assert_exports_map() -> ObjectMap {
    assert_module()
}

fn run_vm(program: &tishlang_ast::Program, path: &Path) -> Result<(), String> {
    let source_arc: Option<Arc<str>> = Some(Arc::from(path.to_string_lossy().as_ref()));
    let mut chunk = tishlang_bytecode::compile_with_source(program, source_arc)
        .map_err(|e| e.to_string())?;
    chunk.source = Some(Arc::from(path.to_string_lossy().as_ref()));

    let mut vm = tishlang_vm::Vm::new();
    vm.register_native_module("tish:test", test_exports_map());
    vm.register_native_module("tish:assert", assert_exports_map());
    vm.register_native_module("node:assert", assert_exports_map());
    vm.register_native_module("node:assert/strict", assert_exports_map());
    if coverage::is_enabled() {
        vm.set_global(Arc::from(hit_global_name()), hit_value());
    }

    let _value = vm.run_with_options(&chunk, false)?;
    if tishlang_core::has_pending_throw() {
        let err = tishlang_core::take_pending_throw().unwrap_or(tishlang_core::Value::Null);
        return Err(format!(
            "Error loading {}: {}",
            path.display(),
            err.to_display_string()
        ));
    }
    Ok(())
}

fn run_interp(program: &tishlang_ast::Program, path: &Path) -> Result<(), String> {
    struct TestNative {
        coverage: bool,
    }
    impl tishlang_eval::TishNativeModule for TestNative {
        fn name(&self) -> &'static str {
            "tish_test"
        }
        fn register(&self) -> std::collections::HashMap<std::sync::Arc<str>, tishlang_eval::Value> {
            let mut map = std::collections::HashMap::new();
            if self.coverage {
                let core = hit_value();
                let v = tishlang_eval::value_convert::core_to_eval(core);
                map.insert(std::sync::Arc::from(hit_global_name()), v);
            }
            map
        }
        fn virtual_builtin_modules(
            &self,
        ) -> Vec<(&'static str, tishlang_eval::Value)> {
            let test = tishlang_eval::value_convert::core_to_eval(
                tishlang_core::Value::object(test_exports_map()),
            );
            let assert = tishlang_eval::value_convert::core_to_eval(
                tishlang_core::Value::object(assert_exports_map()),
            );
            vec![
                ("tish:test", test),
                ("tish:assert", assert.clone()),
                ("node:assert", assert.clone()),
                ("node:assert/strict", assert),
            ]
        }
    }

    let native = TestNative {
        coverage: coverage::is_enabled(),
    };
    let mut eval = tishlang_eval::Evaluator::with_modules(&[&native]);
    eval.set_current_dir(path.parent());
    let _value = eval.eval_program(program)?;
    if tishlang_core::has_pending_throw() {
        let err = tishlang_core::take_pending_throw().unwrap_or(tishlang_core::Value::Null);
        return Err(format!(
            "Error loading {}: {}",
            path.display(),
            err.to_display_string()
        ));
    }
    Ok(())
}
