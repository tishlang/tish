//! Tish CLI - run, REPL, compile to native.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tish")]
#[command(about = "Tish - minimal TS/JS-compatible language")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a Tish file (interpret)
    Run {
        #[arg(required = true)]
        file: String,
        /// Backend: vm (default) or interp (tree-walk interpreter)
        #[arg(long, default_value = "vm")]
        backend: String,
    },
    /// Interactive REPL
    Repl {
        /// Backend: vm (default) or interp (tree-walk interpreter)
        #[arg(long, default_value = "vm")]
        backend: String,
    },
    /// Compile to native binary or JavaScript
    Compile {
        #[arg(required = true)]
        file: String,
        #[arg(short, long, default_value = "tish_out")]
        output: String,
        /// Target: native (default), js, wasm, or wasi
        #[arg(long, default_value = "native")]
        target: String,
        /// Native backend: rust (default), cranelift, or llvm (experimental)
        #[arg(long, default_value = "rust")]
        native_backend: String,
        /// Enable feature (http, fs, process, regex, polars, egui). For native target only. Can be repeated.
        #[arg(long = "feature", action = clap::ArgAction::Append)]
        features: Vec<String>,
    },
    /// Parse and dump AST
    #[command(name = "dump-ast")]
    DumpAst {
        #[arg(required = true)]
        file: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Run { file, backend }) => run_file(&file, &backend),
        Some(Commands::Repl { backend }) => run_repl(&backend),
        Some(Commands::Compile { file, output, target, native_backend, features }) => {
            compile_file(&file, &output, &target, &native_backend, &features)
        }
        Some(Commands::DumpAst { file }) => dump_ast(&file),
        None => run_repl("vm"), // No args = REPL
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_file(path: &str, backend: &str) -> Result<(), String> {
    let path = Path::new(path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", path, e))?;
    let project_root = path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });

    let program = if path.extension().map(|e| e == "js") == Some(true) {
        let source = fs::read_to_string(&path).map_err(|e| format!("{}", e))?;
        js_to_tish::convert(&source).map_err(|e| format!("{}", e))?
    } else {
        let modules = tish_compile::resolve_project(&path, project_root)?;
        tish_compile::detect_cycles(&modules)?;
        tish_compile::merge_modules(modules)?
    };

    if backend == "interp" {
        let mut eval = tish_eval::Evaluator::new();
        let value = eval.eval_program(&program)?;
        if !matches!(value, tish_eval::Value::Null) {
            println!("{}", value);
        }
        return Ok(());
    }

    let chunk = tish_bytecode::compile(&program).map_err(|e| e.to_string())?;
    let value = tish_vm::run(&chunk)?;
    if !matches!(value, tish_core::Value::Null) {
        println!("{}", value.to_display_string());
    }
    Ok(())
}

fn run_repl(backend: &str) -> Result<(), String> {
    println!("Tish REPL (Ctrl-D to exit)");
    let mut buffer = String::new();

    if backend == "interp" {
        let mut eval = tish_eval::Evaluator::new();
        loop {
            print!("> ");
            io::stdout().flush().map_err(|e| e.to_string())?;
            buffer.clear();
            if io::stdin().read_line(&mut buffer).map_err(|e| e.to_string())? == 0 {
                break;
            }
            let line = buffer.trim_end();
            if line.is_empty() {
                continue;
            }
            match tish_parser::parse(line) {
                Ok(program) => {
                    for stmt in &program.statements {
                        if let Ok(v) = eval.eval_program(&tish_ast::Program {
                            statements: vec![stmt.clone()],
                        }) {
                            if !matches!(v, tish_eval::Value::Null) {
                                println!("{}", v);
                            }
                        }
                    }
                }
                Err(e) => eprintln!("Parse error: {}", e),
            }
        }
        return Ok(());
    }

    // VM backend
    let mut vm = tish_vm::Vm::new();
    loop {
        print!("> ");
        io::stdout().flush().map_err(|e| e.to_string())?;
        buffer.clear();
        if io::stdin().read_line(&mut buffer).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let line = buffer.trim_end();
        if line.is_empty() {
            continue;
        }
        match tish_parser::parse(line) {
            Ok(program) => {
                for stmt in &program.statements {
                    let prog = tish_ast::Program {
                        statements: vec![stmt.clone()],
                    };
                    match tish_bytecode::compile(&prog) {
                        Ok(chunk) => {
                            if let Ok(v) = vm.run(&chunk) {
                                if !matches!(v, tish_core::Value::Null) {
                                    println!("{}", v.to_display_string());
                                }
                            }
                        }
                        Err(e) => eprintln!("Compile error: {}", e),
                    }
                }
            }
            Err(e) => eprintln!("Parse error: {}", e),
        }
    }
    Ok(())
}

fn compile_to_js(input_path: &Path, output_path: &str) -> Result<(), String> {
    let project_root = input_path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });
    let js = if input_path.extension().map(|e| e == "js") == Some(true) {
        let source = fs::read_to_string(input_path).map_err(|e| format!("{}", e))?;
        let program = js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        tish_compile_js::compile(&program).map_err(|e| format!("{}", e))?
    } else {
        tish_compile_js::compile_project(input_path, project_root)
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
) -> Result<(), String> {
    let input_path =
        Path::new(input_path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", input_path, e))?;

    let is_js = input_path.extension().map(|e| e == "js") == Some(true);

    if target == "js" {
        return compile_to_js(&input_path, output_path);
    }

    if target == "wasm" && is_js {
        let source = fs::read_to_string(&input_path).map_err(|e| format!("{}", e))?;
        let program = js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        return tish_wasm::compile_program_to_wasm(&program, Path::new(output_path)).map_err(|e| format!("{}", e));
    }

    if target == "wasm" {
        let project_root = input_path.parent().and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("src") {
                p.parent()
            } else {
                Some(p)
            }
        });
        return tish_wasm::compile_to_wasm(&input_path, project_root, Path::new(output_path))
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
        return tish_wasm::compile_to_wasi(&input_path, project_root, Path::new(output_path))
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
        f
    } else {
        cli_features.to_vec()
    };

    if is_js {
        let source = fs::read_to_string(&input_path).map_err(|e| format!("{}", e))?;
        let program = js_to_tish::convert(&source).map_err(|e| format!("{}", e))?;
        tish_native::compile_program_to_native(&program, project_root.as_deref(), Path::new(output_path), &features, native_backend)
            .map_err(|e| e.to_string())?;
    } else {
        tish_native::compile_to_native(&input_path, project_root.as_deref(), Path::new(output_path), &features, native_backend)
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

fn dump_ast(path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    let program = tish_parser::parse(&source)?;
    println!("{:#?}", program);
    Ok(())
}
