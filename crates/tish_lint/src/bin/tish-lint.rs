//! Standalone linter — not part of the `tish` compiler CLI.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};
use serde_json::json;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Text,
    Sarif,
}

#[derive(Parser)]
#[command(name = "tish-lint")]
#[command(about = "AST-based linter for Tish")]
struct Cli {
    /// Output format (SARIF 2.1.0 for code scanning integrations).
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Text)]
    output_format: OutputFormat,

    #[arg(required = true)]
    paths: Vec<String>,
}

#[derive(Debug)]
struct Issue {
    path: PathBuf,
    line: u32,
    col: u32,
    code: String,
    message: String,
    level: &'static str,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli.paths, cli.output_format) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn collect_files(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            for e in walkdir::WalkDir::new(path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if e.path().extension().map(|x| x == "tish").unwrap_or(false) {
                    files.push(e.path().to_path_buf());
                }
            }
        } else if path.exists() {
            files.push(path.to_path_buf());
        } else {
            return Err(format!("Not found: {}", p));
        }
    }
    if files.is_empty() {
        return Err("No .tish files to lint".into());
    }
    Ok(files)
}

fn run(paths: &[String], format: OutputFormat) -> Result<(), String> {
    let files = collect_files(paths)?;
    let mut issues: Vec<Issue> = Vec::new();
    for f in files {
        let src = fs::read_to_string(&f).map_err(|e| format!("{}: {}", f.display(), e))?;
        match tishlang_lint::lint_source(&src) {
            Ok(diags) => {
                for d in diags {
                    let level = match d.severity {
                        tishlang_lint::Severity::Error => "error",
                        tishlang_lint::Severity::Warning => "warning",
                    };
                    issues.push(Issue {
                        path: f.clone(),
                        line: d.line,
                        col: d.col,
                        code: d.code.to_string(),
                        message: d.message,
                        level,
                    });
                }
            }
            Err(e) => {
                issues.push(Issue {
                    path: f.clone(),
                    line: 1,
                    col: 1,
                    code: "tish-parse-error".into(),
                    message: e,
                    level: "error",
                });
            }
        }
    }

    let error_count = issues.iter().filter(|i| i.level == "error").count();

    match format {
        OutputFormat::Text => {
            for i in &issues {
                println!(
                    "{}:{}:{}: {} [{}] {}",
                    i.path.display(),
                    i.line,
                    i.col,
                    i.level,
                    i.code,
                    i.message
                );
            }
            if error_count > 0 {
                return Err(format!("{} issue(s)", error_count));
            }
        }
        OutputFormat::Sarif => {
            print_sarif(&issues)?;
            if error_count > 0 {
                return Err(format!("{} issue(s)", error_count));
            }
        }
    }

    Ok(())
}

fn print_sarif(issues: &[Issue]) -> Result<(), String> {
    let rules: Vec<_> = tishlang_lint::RULES
        .iter()
        .map(|(id, desc)| {
            json!({
                "id": id,
                "name": id,
                "shortDescription": { "text": desc },
                "helpUri": "https://tishlang.com/docs/reference/linting/"
            })
        })
        .chain(std::iter::once(json!({
            "id": "tish-parse-error",
            "name": "tish-parse-error",
            "shortDescription": { "text": "Source failed to parse as Tish." },
            "helpUri": "https://tishlang.com/docs/language/overview/"
        })))
        .collect();

    let results: Vec<_> = issues
        .iter()
        .map(|i| {
            let uri = i.path.to_str().unwrap_or("unknown").replace('\\', "/");
            json!({
                "ruleId": i.code,
                "level": i.level,
                "message": { "text": i.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": uri },
                        "region": {
                            "startLine": i.line,
                            "startColumn": i.col
                        }
                    }
                }]
            })
        })
        .collect();

    let doc = json!({
        "version": "2.1.0",
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "tish-lint",
                    "informationUri": "https://tishlang.com/docs/reference/linting/",
                    "rules": rules
                }
            },
            "results": results
        }]
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?
    );
    Ok(())
}
