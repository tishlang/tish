//! Tish CLI - run, REPL, build to native or other targets.

mod cli_help;
mod repl_completion;

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clap::Parser;
use rustyline::{Behavior, ColorMode, CompletionType, Config, Editor};

use cli_help::{Cli, Commands};

/// Normalize `--feature` / `--feature http,fs` / `--feature full` for VM runs and native builds.
fn normalize_capability_flags(features: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in features {
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "full" {
                for name in ["http", "fs", "process", "regex", "ws"] {
                    out.insert(name.to_string());
                }
            } else {
                out.insert(part.to_string());
            }
        }
    }
    out
}

/// VM capabilities for `run` / `repl` / stdin with the bytecode VM.
///
/// If the user passes no `--feature`, enable **everything linked into this `tish` binary**
/// (so `cargo run --bin tish --features full -- script.tish` does not need `--feature full`).
/// If they pass `--feature …`, use **only** that set (e.g. restrict a full build to `http` only).
fn vm_capabilities_for_cli_run(cli_features: &[String]) -> HashSet<String> {
    if cli_features.is_empty() {
        tishlang_vm::all_compiled_capabilities()
    } else {
        normalize_capability_flags(cli_features)
    }
}

/// `--feature` list for `tish build --target native`: same default as `tish run` (all linked-in caps).
fn native_build_features_from_cli(cli_features: &[String]) -> Vec<String> {
    if cli_features.is_empty() {
        let mut v: Vec<String> = tishlang_vm::all_compiled_capabilities().into_iter().collect();
        v.sort();
        v
    } else {
        cli_features.to_vec()
    }
}

/// `tish script.tish` → insert `run` so it matches `tish run script.tish` (npx / npm UX).
fn argv_with_implicit_run(mut argv: Vec<String>) -> Vec<String> {
    if argv.len() >= 2 {
        let first = argv[1].as_str();
        const SUBCOMMANDS: &[&str] = &["run", "repl", "build", "dump-ast"];
        let looks_like_file =
            !first.starts_with('-') && !SUBCOMMANDS.iter().any(|&s| s == first);
        if looks_like_file {
            argv.insert(1, "run".to_string());
        }
    }
    argv
}

fn main() {
    let no_opt_env = std::env::var_os("TISH_NO_OPTIMIZE")
        .map(|v| v == "1" || v == "true" || v == "yes")
        .unwrap_or(false);

    // `tish -` (like `node -` / `bun -`); clap would treat `-` as an invalid subcommand.
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() == 2 && argv[1] == "-" {
        let result = run_stdin_pipe("vm", &[], no_opt_env, true);
        if let Err(e) = result {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli_help::argv_requests_help(&argv) {
        cli_help::print_tish_banner();
    }

    let argv = argv_with_implicit_run(argv);
    let cli = Cli::parse_from(argv);
    let result = match cli.command {
        Some(Commands::Run(a)) => run_file(&a.file, &a.backend, &a.features, a.no_optimize || no_opt_env),
        Some(Commands::Repl(a)) => run_repl(&a.backend, a.no_optimize || no_opt_env, &a.features),
        Some(Commands::Build(a)) => build_file(
            &a.file,
            &a.output,
            &a.target,
            &a.native_backend,
            &a.features,
            a.no_optimize || no_opt_env,
        ),
        Some(Commands::DumpAst { file }) => dump_ast(&file),
        None => {
            if io::stdin().is_terminal() {
                run_repl("vm", no_opt_env, &[])
            } else {
                // `echo '...' | tish` — run script from stdin (Bun-style)
                run_stdin_pipe("vm", &[], no_opt_env, false)
            }
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

/// Read stdin and run as Tish. If `fail_on_empty`, `tish run -` / `tish -` get an error; if false, empty stdin exits 0.
fn run_stdin_pipe(
    backend: &str,
    features: &[String],
    no_optimize: bool,
    fail_on_empty: bool,
) -> Result<(), String> {
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| format!("Cannot read stdin: {}", e))?;
    if source.trim().is_empty() {
        if fail_on_empty {
            return Err(
                "No source on stdin. Example: echo 'console.log(1)' | tish   or   tish run -".into(),
            );
        }
        return Ok(());
    }
    run_stdin_source(&source, backend, features, no_optimize)
}

fn run_stdin_source(
    source: &str,
    backend: &str,
    features: &[String],
    no_optimize: bool,
) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let modules = tishlang_compile::resolve_project_from_stdin(source, &cwd)?;
    tishlang_compile::detect_cycles(&modules)?;
    let prog = tishlang_compile::merge_modules(modules)?;
    let program = if no_optimize {
        prog
    } else {
        tishlang_opt::optimize(&prog)
    };
    run_program(&program, backend, no_optimize, features)
}

fn run_file(path: &str, backend: &str, features: &[String], no_optimize: bool) -> Result<(), String> {
    let program = if path == "-" {
        return run_stdin_pipe(backend, features, no_optimize, true);
    } else {
        let path =
            Path::new(path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", path, e))?;
        let project_root = path.parent().and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("src") {
                p.parent()
            } else {
                Some(p)
            }
        });

        if path.extension().map(|e| e == "js") == Some(true) {
            let prog = tishlang_js_to_tish::convert(&fs::read_to_string(&path).map_err(|e| format!("{}", e))?)
                .map_err(|e| format!("{}", e))?;
            if no_optimize {
                prog
            } else {
                tishlang_opt::optimize(&prog)
            }
        } else {
            let modules = tishlang_compile::resolve_project(&path, project_root)?;
            tishlang_compile::detect_cycles(&modules)?;
            let prog = tishlang_compile::merge_modules(modules)?;
            if no_optimize {
                prog
            } else {
                tishlang_opt::optimize(&prog)
            }
        }
    };

    run_program(&program, backend, no_optimize, features)
}

fn run_program(
    program: &tishlang_ast::Program,
    backend: &str,
    no_optimize: bool,
    features: &[String],
) -> Result<(), String> {
    if backend == "interp" {
        let mut eval = tishlang_eval::Evaluator::new();
        let value = eval.eval_program(program)?;
        if !matches!(value, tishlang_eval::Value::Null) {
            println!("{}", tishlang_eval::format_value_for_console(&value, tishlang_core::use_console_colors()));
        }
        return Ok(());
    }

    let chunk = if no_optimize {
        tishlang_bytecode::compile_unoptimized(program).map_err(|e| e.to_string())?
    } else {
        tishlang_bytecode::compile(program).map_err(|e| e.to_string())?
    };
    let caps = vm_capabilities_for_cli_run(features);
    let value = tishlang_vm::run_with_options(
        &chunk,
        tishlang_vm::VmRunOptions {
            repl_mode: false,
            capabilities: caps,
        },
    )?;
    if !matches!(value, tishlang_core::Value::Null) {
        println!("{}", tishlang_core::format_value_styled(&value, tishlang_core::use_console_colors()));
    }
    Ok(())
}

fn run_repl(backend: &str, no_optimize: bool, features: &[String]) -> Result<(), String> {
    cli_help::print_tish_banner();
    println!("Tish REPL (Ctrl-D to exit)");
    let mut buffer = String::new();

    if backend == "interp" {
        let mut eval = tishlang_eval::Evaluator::new();
        let mut multiline = String::new();
        loop {
            let prompt = repl_prompt(multiline.is_empty());
            print!("{}", prompt);
            io::stdout().flush().map_err(|e| e.to_string())?;
            buffer.clear();
            if io::stdin().read_line(&mut buffer).map_err(|e| e.to_string())? == 0 {
                if !multiline.is_empty() {
                    let _ = tishlang_parser::parse(multiline.trim());
                }
                break;
            }
            let line = buffer.trim_end();
            if multiline.is_empty() && line.is_empty() {
                continue;
            }
            if multiline.is_empty() {
                multiline = line.to_string();
            } else {
                multiline.push('\n');
                multiline.push_str(line);
            }
            match tishlang_parser::parse(multiline.trim()) {
                Ok(program) => {
                    match eval.eval_program(&program) {
                        Ok(v) => {
                            if !matches!(v, tishlang_eval::Value::Null) {
                                println!("{}", tishlang_eval::format_value_for_console(&v, tishlang_core::use_console_colors()));
                            }
                        }
                        Err(e) => eprintln!("{}", e),
                    }
                    multiline.clear();
                }
                Err(e) => {
                    if e.to_lowercase().contains("eof") {
                        // Incomplete: keep reading
                    } else {
                        eprintln!("Parse error: {}", e);
                        multiline.clear();
                    }
                }
            }
        }
        return Ok(());
    }

    // VM backend with tab completion (e.g. a. -> properties/methods)
    if !std::io::stdin().is_terminal() {
        eprintln!("Note: Tab completion and grey preview require an interactive terminal (TTY).");
    }
    let vm = Rc::new(RefCell::new(tishlang_vm::Vm::with_capabilities(
        vm_capabilities_for_cli_run(features),
    )));
    let completer = repl_completion::ReplCompleter {
        vm: Rc::clone(&vm),
        no_optimize,
    };
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .completion_show_all_if_ambiguous(true)
        .color_mode(ColorMode::Forced)
        .behavior(Behavior::PreferTerm)
        .build();
    let mut rl: Editor<repl_completion::ReplCompleter, _> =
        Editor::with_config(config).map_err(|e| e.to_string())?;
    rl.set_helper(Some(completer));

    if let Some(ref path) = tish_history_path() {
        let _ = rl.load_history(path);
    }

    println!("Tab after 'obj.' for completions (grey preview); press Tab again for full list.");
    println!("Multi-line: type until the statement is complete; use ... continuation prompt.");

    let mut buffer = String::new();

    loop {
        let prompt = repl_prompt(buffer.is_empty());
        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(rustyline::error::ReadlineError::Eof) => {
                if buffer.is_empty() {
                    break;
                }
                match tishlang_parser::parse(buffer.trim()) {
                    Ok(program) => {
                        let compile_fn = if no_optimize {
                            tishlang_bytecode::compile_for_repl_unoptimized
                        } else {
                            tishlang_bytecode::compile_for_repl
                        };
                        if let Ok(chunk) = compile_fn(&program) {
                            let _ = vm.borrow_mut().run_with_options(&chunk, true);
                        }
                    }
                    Err(e) => eprintln!("Parse error: {}", e),
                }
                break;
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                buffer.clear();
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let line = line.trim_end();
        if buffer.is_empty() && line.is_empty() {
            continue;
        }
        if buffer.is_empty() {
            buffer = line.to_string();
        } else {
            buffer.push('\n');
            buffer.push_str(line);
        }
        match tishlang_parser::parse(buffer.trim()) {
            Ok(program) => {
                let compile_fn = if no_optimize {
                    tishlang_bytecode::compile_for_repl_unoptimized
                } else {
                    tishlang_bytecode::compile_for_repl
                };
                match compile_fn(&program) {
                    Ok(chunk) => {
                        match vm.borrow_mut().run_with_options(&chunk, true) {
                            Ok(v) => {
                                if !matches!(v, tishlang_core::Value::Null) {
                                    println!("{}", tishlang_core::format_value_styled(&v, tishlang_core::use_console_colors()));
                                }
                            }
                            Err(e) => eprintln!("{}", e),
                        }
                    }
                    Err(e) => eprintln!("Compile error: {}", e),
                }
                let _ = rl.add_history_entry(buffer.trim());
                buffer.clear();
            }
            Err(e) => {
                if e.to_lowercase().contains("eof") {
                    // Incomplete: keep accumulating (Python-style ... prompt)
                } else {
                    eprintln!("Parse error: {}", e);
                    buffer.clear();
                }
            }
        }
    }

    if let Some(ref path) = tish_history_path() {
        let _ = rl.save_history(path);
    }
    Ok(())
}

/// REPL prompt with green caret when stdout is a TTY (platform-style).
fn repl_prompt(primary: bool) -> String {
    if tishlang_core::use_console_colors() {
        if primary {
            "\x1b[32m> \x1b[0m".to_string()
        } else {
            "\x1b[32m... \x1b[0m".to_string()
        }
    } else if primary {
        "> ".to_string()
    } else {
        "... ".to_string()
    }
}

/// Path to REPL history file (Python-style: ~/.tish_history).
fn tish_history_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"));
    home.map(|h| PathBuf::from(h).join(".tish_history"))
}

fn compile_to_js(input_path: &Path, output_path: &str, optimize: bool) -> Result<(), String> {
    let project_root = input_path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });
    let js = if input_path.extension().map(|e| e == "jsx") == Some(true) {
        let source = fs::read_to_string(input_path).map_err(|e| format!("{}", e))?;
        let wrapped = format!(
            "export fn __TishJsxRoot() {{\n  return (\n{}\n  )\n}}",
            source.trim()
        );
        let program = tishlang_parser::parse(&wrapped)
            .map_err(|e| format!("JSX wrapper parse: {}", e))?;
        let p = if optimize {
            tishlang_opt::optimize(&program)
        } else {
            program
        };
        tishlang_compile_js::compile_with_jsx(&p, optimize).map_err(|e| format!("{}", e))?
    } else if input_path.extension().map(|e| e == "js") == Some(true) {
        let source = fs::read_to_string(input_path).map_err(|e| format!("{}", e))?;
        let program = tishlang_js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        tishlang_compile_js::compile_with_jsx(&program, optimize).map_err(|e| format!("{}", e))?
    } else {
        tishlang_compile_js::compile_project_with_jsx(input_path, project_root, optimize)
            .map_err(|e| format!("{}", e))?
    };

    let out_path = Path::new(output_path);
    let out_path = if out_path.extension().is_none()
        || out_path.extension() == Some(std::ffi::OsStr::new(""))
    {
        out_path.with_extension("js")
    } else {
        out_path.to_path_buf()
    };

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create output directory {}: {}", parent.display(), e))?;
    }
    fs::write(&out_path, js).map_err(|e| format!("Cannot write {}: {}", out_path.display(), e))?;
    println!("Built: {}", out_path.display());
    Ok(())
}

#[allow(clippy::vec_init_then_push)]
fn build_file(
    input_path: &str,
    output_path: &str,
    target: &str,
    native_backend: &str,
    cli_features: &[String],
    no_optimize: bool,
) -> Result<(), String> {
    let optimize = !no_optimize;
    let input_path =
        Path::new(input_path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", input_path, e))?;

    let is_js = input_path.extension().map(|e| e == "js") == Some(true);

    if target == "js" {
        return compile_to_js(&input_path, output_path, optimize);
    }

    if target == "wasm" && is_js {
        let source = fs::read_to_string(&input_path).map_err(|e| format!("{}", e))?;
        let program = tishlang_js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        return tishlang_wasm::compile_program_to_wasm(&program, Path::new(output_path), optimize)
            .map_err(|e| format!("{}", e));
    }

    if target == "wasm" {
        let project_root = input_path.parent().and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("src") {
                p.parent()
            } else {
                Some(p)
            }
        });
        return tishlang_wasm::compile_to_wasm(&input_path, project_root, Path::new(output_path), optimize)
            .map_err(|e| e.to_string());
    }

    if target == "wasi" {
        let project_root = input_path.parent().and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("src") {
                p.parent()
            } else {
                Some(p)
            }
        });
        return tishlang_wasm::compile_to_wasi(&input_path, project_root, Path::new(output_path), optimize)
            .map_err(|e| e.to_string());
    }

    if target != "native" {
        return Err(format!(
            "Unknown target: {}. Use 'native', 'js', 'wasm', or 'wasi'.",
            target
        ));
    }

    let project_root = input_path.parent().map(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent().unwrap_or(p)
        } else {
            p
        }
    });
    let features: Vec<String> = native_build_features_from_cli(cli_features);

    if is_js {
        let source = fs::read_to_string(&input_path).map_err(|e| format!("{}", e))?;
        let program = tishlang_js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        tishlang_native::compile_program_to_native(
            &program,
            project_root,
            Path::new(output_path),
            &features,
            native_backend,
            optimize,
        )
        .map_err(|e| e.to_string())?;
    } else {
        tishlang_native::compile_to_native(
            &input_path,
            project_root,
            Path::new(output_path),
            &features,
            native_backend,
            optimize,
        )
        .map_err(|e| e.to_string())?;
    }

    let out_name = Path::new(output_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let built_path = if output_path.ends_with('/') || Path::new(output_path).is_dir() {
        Path::new(output_path).join(out_name)
    } else {
        Path::new(output_path).to_path_buf()
    };
    println!("Built: {}", built_path.display());
    Ok(())
}



#[cfg(test)]
mod cli_tests {
    use clap::Parser;

    use crate::cli_help::{Cli, Commands};

    use super::argv_with_implicit_run;

    #[test]
    fn implicit_run_inserts_run_before_file() {
        let argv = argv_with_implicit_run(vec![
            "tish".to_string(),
            "hello.tish".to_string(),
        ]);
        let cli = Cli::try_parse_from(argv).unwrap();
        match cli.command {
            Some(Commands::Run(a)) => assert_eq!(a.file, "hello.tish"),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn explicit_subcommand_not_treated_as_file() {
        let argv = argv_with_implicit_run(vec![
            "tish".to_string(),
            "repl".to_string(),
        ]);
        let cli = Cli::try_parse_from(argv).unwrap();
        assert!(matches!(cli.command, Some(Commands::Repl(_))));
    }

    #[test]
    fn build_js_target_parses() {
        let cli = Cli::try_parse_from([
            "tish",
            "build",
            "m.tish",
            "--target",
            "js",
            "-o",
            "x.js",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Build(a)) => assert_eq!(a.file, "m.tish"),
            _ => panic!("expected Build"),
        }
    }

    #[test]
    fn run_stdin_marker_parses_as_file() {
        let cli = Cli::try_parse_from(["tish", "run", "-"]).unwrap();
        match cli.command {
            Some(Commands::Run(a)) => assert_eq!(a.file, "-"),
            _ => panic!("expected Run"),
        }
    }
}

fn dump_ast(path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    let program = tishlang_parser::parse(&source)?;
    println!("{:#?}", program);
    Ok(())
}
