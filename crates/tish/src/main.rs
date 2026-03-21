//! Tish CLI - run, REPL, compile to native.

mod repl_completion;

use std::cell::RefCell;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clap::{Parser, Subcommand};
use rustyline::{Behavior, ColorMode, CompletionType, Config, Editor};

#[derive(Parser)]
#[command(name = "tish")]
#[command(about = "Tish - minimal TS/JS-compatible language")]
#[command(after_help = "To disable optimizations: TISH_NO_OPTIMIZE=1")]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Parser)]
struct RunArgs {
    #[arg(required = true)]
    file: String,
    #[arg(long, default_value = "vm")]
    backend: String,
    /// Enable capabilities (http, fs, process, regex, ws). Must match how tish was built.
    /// E.g. cargo run -p tish --features http,fs -- run script.tish --feature http,fs
    #[arg(long = "feature", action = clap::ArgAction::Append)]
    features: Vec<String>,
    /// Disable AST and bytecode optimizations (for debugging)
    #[arg(long)]
    no_optimize: bool,
}

#[derive(Parser)]
struct ReplArgs {
    #[arg(long, default_value = "vm")]
    backend: String,
    #[arg(long)]
    no_optimize: bool,
}

#[derive(Parser)]
struct CompileArgs {
    #[arg(short, long, default_value = "tish_out")]
    output: String,
    #[arg(long, default_value = "native")]
    target: String,
    #[arg(long, default_value = "rust")]
    native_backend: String,
    #[arg(long = "feature", action = clap::ArgAction::Append)]
    features: Vec<String>,
    #[arg(long)]
    no_optimize: bool,
    /// JS target only: `lattish` (default), `vdom` (vnode + patch; use with Lattish createRoot).
    #[arg(long = "jsx", value_name = "MODE", default_value = "lattish")]
    jsx_mode: String,
    #[arg(required = true)]
    file: String,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Run a Tish file (interpret)
    Run(RunArgs),
    /// Interactive REPL
    Repl(ReplArgs),
    /// Compile to native binary or JavaScript
    Compile(CompileArgs),
    /// Parse and dump AST
    #[command(name = "dump-ast")]
    DumpAst {
        #[arg(required = true)]
        file: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let no_opt_env = std::env::var_os("TISH_NO_OPTIMIZE")
        .map(|v| v == "1" || v == "true" || v == "yes")
        .unwrap_or(false);
    let result = match cli.command {
        Some(Commands::Run(a)) => run_file(&a.file, &a.backend, &a.features, a.no_optimize || no_opt_env),
        Some(Commands::Repl(a)) => run_repl(&a.backend, a.no_optimize || no_opt_env),
        Some(Commands::Compile(a)) => compile_file(
            &a.file,
            &a.output,
            &a.target,
            &a.native_backend,
            &a.features,
            a.no_optimize || no_opt_env,
            &a.jsx_mode,
        ),
        Some(Commands::DumpAst { file }) => dump_ast(&file),
        None => run_repl("vm", false), // No args = REPL
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_file(path: &str, backend: &str, _features: &[String], no_optimize: bool) -> Result<(), String> {
    let path = Path::new(path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", path, e))?;
    let project_root = path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });

    let program = if path.extension().map(|e| e == "js") == Some(true) {
        let prog = js_to_tish::convert(&fs::read_to_string(&path).map_err(|e| format!("{}", e))?)
            .map_err(|e| format!("{}", e))?;
        if no_optimize {
            prog
        } else {
            tish_opt::optimize(&prog)
        }
    } else {
        let modules = tish_compile::resolve_project(&path, project_root)?;
        tish_compile::detect_cycles(&modules)?;
        let prog = tish_compile::merge_modules(modules)?;
        if no_optimize {
            prog
        } else {
            tish_opt::optimize(&prog)
        }
    };

    if backend == "interp" {
        let mut eval = tish_eval::Evaluator::new();
        let value = eval.eval_program(&program)?;
        if !matches!(value, tish_eval::Value::Null) {
            println!("{}", tish_eval::format_value_for_console(&value, tish_core::use_console_colors()));
        }
        return Ok(());
    }

    // VM backend (bytecode) - supports native imports when built with fs/http/process features
    let chunk = if no_optimize {
        tish_bytecode::compile_unoptimized(&program).map_err(|e| e.to_string())?
    } else {
        tish_bytecode::compile(&program).map_err(|e| e.to_string())?
    };
    let value = tish_vm::run(&chunk)?;
    if !matches!(value, tish_core::Value::Null) {
        println!("{}", tish_core::format_value_styled(&value, tish_core::use_console_colors()));
    }
    Ok(())
}

fn run_repl(backend: &str, no_optimize: bool) -> Result<(), String> {
    println!("Tish REPL (Ctrl-D to exit)");
    let mut buffer = String::new();

    if backend == "interp" {
        let mut eval = tish_eval::Evaluator::new();
        let mut multiline = String::new();
        loop {
            let prompt = repl_prompt(multiline.is_empty());
            print!("{}", prompt);
            io::stdout().flush().map_err(|e| e.to_string())?;
            buffer.clear();
            if io::stdin().read_line(&mut buffer).map_err(|e| e.to_string())? == 0 {
                if !multiline.is_empty() {
                    let _ = tish_parser::parse(multiline.trim());
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
            match tish_parser::parse(multiline.trim()) {
                Ok(program) => {
                    match eval.eval_program(&program) {
                        Ok(v) => {
                            if !matches!(v, tish_eval::Value::Null) {
                                println!("{}", tish_eval::format_value_for_console(&v, tish_core::use_console_colors()));
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
    let vm = Rc::new(RefCell::new(tish_vm::Vm::new()));
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
                match tish_parser::parse(buffer.trim()) {
                    Ok(program) => {
                        let compile_fn = if no_optimize {
                            tish_bytecode::compile_for_repl_unoptimized
                        } else {
                            tish_bytecode::compile_for_repl
                        };
                        if let Ok(chunk) = compile_fn(&program) {
                            let _ = vm.borrow_mut().run(&chunk);
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
        match tish_parser::parse(buffer.trim()) {
            Ok(program) => {
                let compile_fn = if no_optimize {
                    tish_bytecode::compile_for_repl_unoptimized
                } else {
                    tish_bytecode::compile_for_repl
                };
                match compile_fn(&program) {
                    Ok(chunk) => {
                        match vm.borrow_mut().run(&chunk) {
                            Ok(v) => {
                                if !matches!(v, tish_core::Value::Null) {
                                    println!("{}", tish_core::format_value_styled(&v, tish_core::use_console_colors()));
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
    if tish_core::use_console_colors() {
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

fn parse_jsx_mode(s: &str) -> Result<tish_compile_js::JsxMode, String> {
    match s {
        "legacy" => Err(
            "--jsx legacy was removed. Use --jsx lattish (default) with lattish merged into your \
             bundle, or --jsx vdom with Lattish's createRoot."
                .to_string(),
        ),
        "vdom" => Ok(tish_compile_js::JsxMode::Vdom),
        "lattish" => Ok(tish_compile_js::JsxMode::LattishH),
        other => Err(format!(
            "Unknown --jsx {:?}: use lattish (default) or vdom.",
            other
        )),
    }
}

fn compile_to_js(
    input_path: &Path,
    output_path: &str,
    optimize: bool,
    jsx: &str,
) -> Result<(), String> {
    let jsx_mode = parse_jsx_mode(jsx)?;
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
        let program = tish_parser::parse(&wrapped)
            .map_err(|e| format!("JSX wrapper parse: {}", e))?;
        let p = if optimize {
            tish_opt::optimize(&program)
        } else {
            program
        };
        tish_compile_js::compile_with_jsx(&p, optimize, jsx_mode).map_err(|e| format!("{}", e))?
    } else if input_path.extension().map(|e| e == "js") == Some(true) {
        let source = fs::read_to_string(input_path).map_err(|e| format!("{}", e))?;
        let program = js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        tish_compile_js::compile_with_jsx(&program, optimize, jsx_mode).map_err(|e| format!("{}", e))?
    } else {
        tish_compile_js::compile_project_with_jsx(input_path, project_root, optimize, jsx_mode)
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
fn compile_file(
    input_path: &str,
    output_path: &str,
    target: &str,
    native_backend: &str,
    cli_features: &[String],
    no_optimize: bool,
    jsx: &str,
) -> Result<(), String> {
    let optimize = !no_optimize;
    let input_path =
        Path::new(input_path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", input_path, e))?;

    let is_js = input_path.extension().map(|e| e == "js") == Some(true);

    if target == "js" {
        return compile_to_js(&input_path, output_path, optimize, jsx.trim());
    }

    if target == "wasm" && is_js {
        let source = fs::read_to_string(&input_path).map_err(|e| format!("{}", e))?;
        let program = js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        return tish_wasm::compile_program_to_wasm(&program, Path::new(output_path), optimize)
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
        return tish_wasm::compile_to_wasm(&input_path, project_root, Path::new(output_path), optimize)
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
        return tish_wasm::compile_to_wasi(&input_path, project_root, Path::new(output_path), optimize)
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
    let features: Vec<String> = if cli_features.is_empty() {
        #[allow(unused_mut)]
        let mut f = Vec::new();
        #[cfg(feature = "http")]
        f.push("http".to_string());
        #[cfg(feature = "fs")]
        f.push("fs".to_string());
        #[cfg(feature = "process")]
        f.push("process".to_string());
        #[cfg(feature = "regex")]
        f.push("regex".to_string());
        #[cfg(feature = "ws")]
        f.push("ws".to_string());
        f
    } else {
        cli_features.to_vec()
    };

    if is_js {
        let source = fs::read_to_string(&input_path).map_err(|e| format!("{}", e))?;
        let program = js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        tish_native::compile_program_to_native(
            &program,
            project_root,
            Path::new(output_path),
            &features,
            native_backend,
            optimize,
        )
        .map_err(|e| e.to_string())?;
    } else {
        tish_native::compile_to_native(
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

    use super::{Cli, Commands};

    #[test]
    fn compile_jsx_defaults_to_lattish() {
        let cli = Cli::try_parse_from([
            "tish",
            "compile",
            "m.tish",
            "--target",
            "js",
            "-o",
            "x.js",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Compile(a)) => assert_eq!(a.jsx_mode.as_str(), "lattish"),
            _ => panic!("expected Compile"),
        }
    }

    #[test]
    fn compile_jsx_flag_vdom() {
        let cli = Cli::try_parse_from([
            "tish",
            "compile",
            "a.tish",
            "--target",
            "js",
            "--jsx",
            "vdom",
            "-o",
            "x.js",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Compile(a)) => assert_eq!(a.jsx_mode.as_str(), "vdom"),
            _ => panic!("expected Compile"),
        }
    }
}

fn dump_ast(path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    let program = tish_parser::parse(&source)?;
    println!("{:#?}", program);
    Ok(())
}
