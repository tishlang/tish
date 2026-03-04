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
    /// Compile to native binary
    Compile {
        #[arg(required = true)]
        file: String,
        #[arg(short, long, default_value = "tish_out")]
        output: String,
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
        Some(Commands::Compile { file, output }) => compile_file(&file, &output),
        Some(Commands::DumpAst { file }) => dump_ast(&file),
        None => run_repl(), // No args = REPL
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_file(path: &str) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    let value = tish_eval::run(&source)?;
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
fn compile_file(input_path: &str, output_path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(input_path).map_err(|e| format!("Cannot read {}: {}", input_path, e))?;
    let program = tish_parser::parse(&source)?;
    let rust_code = tish_compile::compile(&program).map_err(|e| {
        if let Some(ref span) = e.span {
            format!("{}:{}:{}: {}", input_path, span.start.0, span.start.1, e.message)
        } else {
            format!("{}: {}", input_path, e.message)
        }
    })?;

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

    #[allow(unused_mut)]
    let mut features: Vec<&str> = Vec::new();
    #[cfg(feature = "http")]
    features.push("http");
    #[cfg(feature = "fs")]
    features.push("fs");
    #[cfg(feature = "process")]
    features.push("process");
    #[cfg(feature = "regex")]
    features.push("regex");
    let features_str = if features.is_empty() {
        String::new()
    } else {
        format!(", features = {:?}", features)
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
tish_runtime = {{ path = {:?}{} }}
"#,
        out_name, runtime_path, features_str
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;
    fs::write(build_dir.join("src/main.rs"), rust_code)
        .map_err(|e| format!("Cannot write main.rs: {}", e))?;

    let target_dir = build_dir.join("target");
    let status = Command::new("cargo")
        .args(["build", "--release", "--target-dir"])
        .arg(&target_dir)
        .current_dir(&build_dir)
        .env_remove("CARGO_TARGET_DIR") // use our explicit target-dir
        .status()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if !status.success() {
        return Err("Compilation failed".to_string());
    }

    let binary_no_ext = build_dir
        .join("target")
        .join("release")
        .join(out_name);
    let binary_exe = build_dir
        .join("target")
        .join("release")
        .join(format!("{}.exe", out_name));
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
