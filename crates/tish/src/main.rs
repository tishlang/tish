//! Tish CLI - run, REPL, compile to native.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

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
    },
    /// Interactive REPL
    Repl,
    /// Compile to native binary or JavaScript
    Compile {
        #[arg(required = true)]
        file: String,
        #[arg(short, long, default_value = "tish_out")]
        output: String,
        /// Target: native (default) or js
        #[arg(long, default_value = "native")]
        target: String,
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
        Some(Commands::Run { file }) => run_file(&file),
        Some(Commands::Repl) => run_repl(),
        Some(Commands::Compile { file, output, target, features }) => {
            compile_file(&file, &output, &target, &features)
        }
        Some(Commands::DumpAst { file }) => dump_ast(&file),
        None => run_repl(), // No args = REPL
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_file(path: &str) -> Result<(), String> {
    let path = Path::new(path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", path, e))?;
    let project_root = path.parent().and_then(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent()
        } else {
            Some(p)
        }
    });
    let value = tish_eval::run_file(&path, project_root)?;
    if !matches!(value, tish_eval::Value::Null) {
        println!("{}", value);
    }
    Ok(())
}

fn run_repl() -> Result<(), String> {
    println!("Tish REPL (Ctrl-D to exit)");
    let mut buffer = String::new();
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
    let js = tish_compile_js::compile_project(input_path, project_root)
        .map_err(|e| format!("{}", e))?;

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

/// Find the tish_runtime crate path using multiple strategies
fn find_runtime_path() -> Result<String, String> {
    // Strategy 1: CARGO_MANIFEST_DIR (works during cargo run/build)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = Path::new(&manifest_dir).join("..").join("tish_runtime");
        if let Ok(canonical) = path.canonicalize() {
            return Ok(canonical.display().to_string().replace('\\', "/"));
        }
    }

    // Strategy 2: Relative to executable location (target/debug/tish or target/release/tish)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let path = exe_dir.join("..").join("..").join("crates").join("tish_runtime");
            if let Ok(canonical) = path.canonicalize() {
                return Ok(canonical.display().to_string().replace('\\', "/"));
            }
        }
    }

    // Strategy 3: Current working directory based (common dev scenario)
    let cwd_based = Path::new("crates").join("tish_runtime");
    if let Ok(canonical) = cwd_based.canonicalize() {
        return Ok(canonical.display().to_string().replace('\\', "/"));
    }

    // Strategy 4: Look for Cargo.toml to find workspace root
    if let Ok(mut current) = std::env::current_dir() {
        for _ in 0..10 {
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                let runtime = current.join("crates").join("tish_runtime");
                if runtime.exists() {
                    return Ok(runtime.display().to_string().replace('\\', "/"));
                }
            }
            if !current.pop() {
                break;
            }
        }
    }

    Err("Could not find tish_runtime crate. Run from workspace root or use cargo run.".to_string())
}

#[allow(clippy::vec_init_then_push)]
fn compile_file(
    input_path: &str,
    output_path: &str,
    target: &str,
    cli_features: &[String],
) -> Result<(), String> {
    let input_path =
        Path::new(input_path).canonicalize().map_err(|e| format!("Cannot resolve {}: {}", input_path, e))?;

    if target == "js" {
        return compile_to_js(&input_path, output_path);
    }

    if target != "native" {
        return Err(format!("Unknown target: {}. Use 'native' or 'js'.", target));
    }
    let project_root = input_path.parent().map(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent().unwrap_or(p)
        } else {
            p
        }
    });
    let features: Vec<String> = if cli_features.is_empty() {
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
    let (rust_code, native_modules) = tish_compile::compile_project_full(&input_path, project_root, &features).map_err(|e| {
        if let Some(ref span) = e.span {
            format!("{}:{}:{}: {}", input_path.display(), span.start.0, span.start.1, e.message)
        } else {
            format!("{}: {}", input_path.display(), e.message)
        }
    })?;

    let native_deps: String = native_modules
        .iter()
        .map(|m| {
            let path = m.crate_path.display().to_string().replace('\\', "/");
            format!("{} = {{ path = {:?} }}\n", m.package_name, path)
        })
        .collect();

    let out_name = Path::new(output_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let build_dir = std::env::temp_dir().join(format!("tish_build_{}_{}", out_name, std::process::id()));

    fs::create_dir_all(&build_dir).map_err(|e| format!("Cannot create build dir: {}", e))?;
    fs::create_dir_all(build_dir.join("src")).map_err(|e| format!("Cannot create src: {}", e))?;

    // Path to tish_runtime: try multiple strategies to locate it
    let runtime_path = find_runtime_path()
        .map_err(|e| format!("Cannot resolve tish_runtime path: {}", e))?;

    let runtime_features: Vec<&str> = features
        .iter()
        .filter(|f| ["http", "fs", "process", "regex"].contains(&f.as_str()))
        .map(|s| s.as_str())
        .collect();
    let features_str = if runtime_features.is_empty() {
        String::new()
    } else {
        format!(", features = {:?}", runtime_features)
    };

    let needs_tokio = rust_code.contains("#[tokio::main]");
    let tokio_dep = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }\n"
    } else {
        ""
    };

    let cargo_toml = format!(
        r#"[package]
name = "tish_output"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{}"
path = "src/main.rs"

[dependencies]
tish_runtime = {{ path = {:?}{} }}{}{}
"#,
        out_name, runtime_path, features_str, tokio_dep,
        if native_deps.is_empty() { String::new() } else { format!("\n{}", native_deps) }
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;
    fs::write(build_dir.join("src/main.rs"), rust_code)
        .map_err(|e| format!("Cannot write main.rs: {}", e))?;

    // Use workspace target dir when possible for cache reuse (avoids slow "Updating crates.io index")
    let workspace_target = Path::new(&runtime_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|ws| ws.join("target"));
    let (target_dir, binary_dir) = if let Some(ref wt) = workspace_target.filter(|p| p.exists()) {
        (wt.clone(), wt.join("release"))
    } else {
        let td = build_dir.join("target");
        (td.clone(), td.join("release"))
    };

    let status = Command::new("cargo")
        .args(["build", "--release", "--target-dir"])
        .arg(&target_dir)
        .current_dir(&build_dir)
        .env_remove("CARGO_TARGET_DIR")
        .env("CARGO_TERM_PROGRESS", "always") // Ensure progress is streamed (avoids "stuck" appearance)
        .status()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if !status.success() {
        return Err("Compilation failed".to_string());
    }

    let binary_no_ext = binary_dir.join(out_name);
    let binary_exe = binary_dir.join(format!("{}.exe", out_name));
    let binary = if binary_no_ext.exists() {
        binary_no_ext
    } else if binary_exe.exists() {
        binary_exe
    } else {
        return Err(format!(
            "Binary not found at {} or {}",
            binary_no_ext.display(),
            binary_exe.display()
        ));
    };
    let target = if output_path.ends_with('/') || Path::new(output_path).is_dir() {
        Path::new(output_path).join(out_name)
    } else {
        Path::new(output_path).to_path_buf()
    };

    fs::copy(&binary, &target)
        .map_err(|e| format!("Cannot copy {} to {}: {}", binary.display(), target.display(), e))?;

    println!("Built: {}", target.display());
    Ok(())
}

fn dump_ast(path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    let program = tish_parser::parse(&source)?;
    println!("{:#?}", program);
    Ok(())
}
