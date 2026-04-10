//! Long help text, terminal styling, and ASCII banner for the `tish` CLI.

use std::io::{self, IsTerminal, Write};
use std::thread;
use std::time::Duration;

use clap::builder::styling::{Color, Effects, RgbColor, Style, Styles};
use clap::{CommandFactory, Parser, Subcommand};

/// FIGlet-style block letters (UTF-8). On a TTY, a short expand + palette-color animation runs.
const TISH_BANNER_LINES: &[&str] = &[
        "",
    "████████╗██╗███████╗██╗  ██╗",
    "╚══██╔══╝██║██╔════╝██║  ██║",
    "   ██║   ██║███████╗███████║",
    "   ██║   ██║╚════██║██╔══██║",
    "   ██║   ██║███████║██║  ██║",
    "   ╚═╝   ╚═╝╚══════╝╚═╝  ╚═╝",
];

/// Frames used for the left-to-right expand reveal.
const BANNER_REVEAL_FRAMES: usize = 14;
/// Extra frames of rainbow cycling after the logo is fully visible.
const BANNER_CYCLE_FRAMES: usize = 4;
const BANNER_FRAME_MS: u64 = 20;

/// Orange → Yellow → Green → Teal → Blue → Purple → Pink (matching the brand palette).
const PALETTE: &[(u8, u8, u8)] = &[
    (255, 159,  64),  // Orange
    (255, 213,  64),  // Yellow
    ( 52, 199,  89),  // Green
    ( 48, 209, 188),  // Teal
    ( 10, 132, 255),  // Blue
    (175,  82, 222),  // Purple
    (255,  55, 148),  // Pink
];

fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Linearly interpolate between two palette colors.
fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t).round() as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t).round() as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t).round() as u8,
    )
}

/// Smooth palette sample for a given (row, col) cell and scrolling color frame.
/// Uses column as the primary gradient axis so every row has a continuous sweep.
/// A small per-row offset adds a gentle diagonal tilt rather than flat stripes.
fn palette_color(row: usize, col: usize, color_frame: usize) -> (u8, u8, u8) {
    let n = PALETTE.len();
    // one full palette cycle every ~5 columns; row adds a slight diagonal
    let scroll = color_frame as f32 * 0.22;
    let pos = ((col as f32 / 5.0) + (row as f32 * 0.25) + scroll).rem_euclid(n as f32);
    let lo = pos.floor() as usize % n;
    let hi = (lo + 1) % n;
    lerp_color(PALETTE[lo], PALETTE[hi], pos.fract())
}

/// Render one frame. `reveal_t` is 0..=1 (how much of each line is visible).
/// `color_frame` is the ever-incrementing counter that drives the rainbow scroll.
fn write_tish_banner_frame(out: &mut impl Write, reveal_t: f32, color_frame: usize) {
    for (row, line) in TISH_BANNER_LINES.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let visible = ((len as f32) * reveal_t).round() as usize;
        let visible = visible.min(len);

        for col in 0..len {
            let ch = chars[col];
            if col >= visible || ch == ' ' {
                let _ = write!(out, " ");
            } else {
                let (r, g, b) = palette_color(row, col, color_frame);
                let _ = write!(out, "\x1b[1;38;2;{r};{g};{b}m{ch}\x1b[0m");
            }
        }
        let _ = writeln!(out);
    }
}

fn print_tish_banner_plain(out: &mut impl Write) {
    for line in TISH_BANNER_LINES {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out);
}

fn print_tish_banner_animated(out: &mut impl Write) {
    let n = TISH_BANNER_LINES.len();
    let total = BANNER_REVEAL_FRAMES + BANNER_CYCLE_FRAMES;

    for f in 0..total {
        if f > 0 {
            let _ = write!(out, "\x1b[{n}A");
        }
        // Phase 1: ease-out expand.  Phase 2: fully visible, rainbow keeps scrolling.
        let reveal_t = if f < BANNER_REVEAL_FRAMES {
            ease_out_cubic((f + 1) as f32 / BANNER_REVEAL_FRAMES as f32)
        } else {
            1.0
        };
        write_tish_banner_frame(out, reveal_t, f);
        let _ = out.flush();
        thread::sleep(Duration::from_millis(BANNER_FRAME_MS));
    }
    let _ = writeln!(out);
}

/// Print the `TISH` tile banner to stdout (animated palette on a TTY; plain text otherwise).
pub fn print_tish_banner() {
    let mut out = io::stdout().lock();
    if io::stdout().is_terminal() {
        print_tish_banner_animated(&mut out);
    } else {
        print_tish_banner_plain(&mut out);
    }
}

/// Build the `Command` with all colored after_help text attached.
/// Use this instead of `Cli::command()` everywhere so the help text is consistent.
pub fn build_command() -> clap::Command {
    Cli::command()
        .after_help(cli_after_help())
        .mut_subcommand("run",   |sub| sub.after_help(run_after_help()))
        .mut_subcommand("repl",  |sub| sub.after_help(repl_after_help()))
        .mut_subcommand("build", |sub| sub.after_long_help(build_after_help()))
}

/// Write help text to `w` (plain bytes, used for line-counting only).
fn count_help_lines(cmd: &mut clap::Command, sub_name: Option<&str>) -> usize {
    let mut buf = Vec::<u8>::new();
    if let Some(name) = sub_name {
        if cmd.find_subcommand(name).is_some() {
            let _ = cmd.find_subcommand_mut(name).unwrap().write_long_help(&mut buf);
        } else {
            let _ = cmd.write_long_help(&mut buf);
        }
    } else {
        let _ = cmd.write_long_help(&mut buf);
    }
    buf.iter().filter(|&&b| b == b'\n').count()
}

/// Print help text directly to stdout via clap's own stdout path (guaranteed colors).
fn print_help_to_stdout(cmd: &mut clap::Command, sub_name: Option<&str>) {
    if let Some(name) = sub_name {
        if cmd.find_subcommand(name).is_some() {
            let _ = cmd.find_subcommand_mut(name).unwrap().print_long_help();
            return;
        }
    }
    let _ = cmd.print_long_help();
}

/// Detect which subcommand (if any) is being asked about from raw argv.
fn sub_name_from_argv(argv: &[String]) -> Option<String> {
    match argv.get(1).map(String::as_str) {
        Some("help") => argv.get(2).map(String::to_string), // tish help run
        Some(s) if !s.starts_with('-') => Some(s.to_string()), // tish run --help
        _ => None,
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// ANSI: bold purple (175, 82, 222)
const H_PURPLE: &str = "\x1b[1;38;2;175;82;222m";
/// ANSI: medium grey — clearly less-than-white on dark backgrounds
const H_GREY: &str = "\x1b[38;2;150;150;150m";
/// ANSI: pink (255, 55, 148) — used for the website URL
const H_PINK: &str = "\x1b[38;2;255;55;148m";
const H_RESET: &str = "\x1b[0m";

/// Branded header used for subcommand help pages.
/// Prints  `[purple]Tish[reset] [grey](version x)[reset]`
///         `[pink]https://tishlang.com[reset]`
/// followed by a blank line.
fn print_small_header() {
    if io::stdout().is_terminal() {
        println!("{H_PURPLE}Tish{H_RESET} {H_GREY}(version {VERSION}){H_RESET}");
        println!("{H_PINK}https://tishlang.com{H_RESET}\n");
    } else {
        println!("Tish (version {VERSION})");
        println!("https://tishlang.com\n");
    }
}

/// Number of lines the main-help manual prefix takes (printed before clap output).
/// Layout: title, description, url, blank = 4 lines.
const MAIN_PREFIX_LINES: usize = 4;

/// Print help, prefixed with the right header and (for top-level only) the
/// animated banner.  Help is written via clap's own stdout path for full colors.
pub fn print_banner_with_help(argv: &[String]) {
    let sub_name = sub_name_from_argv(argv);
    let sub = sub_name.as_deref();

    // ── Subcommand help: compact static header, no animation ─────────────
    if sub.is_some() {
        print_small_header();
        let mut cmd = build_command();
        cmd.build();
        print_help_to_stdout(&mut cmd, sub);
        return;
    }

    // ── Top-level help ────────────────────────────────────────────────────

    if !io::stdout().is_terminal() {
        let mut out = io::stdout().lock();
        print_tish_banner_plain(&mut out);
        drop(out);
        let mut cmd = build_command().color(clap::ColorChoice::Never);
        cmd.build();
        print_help_to_stdout(&mut cmd, sub);
        return;
    }

    // Line-count pass (ANSI codes never add \n, so Never == Always count).
    let h: usize = {
        let mut cmd = build_command().color(clap::ColorChoice::Never);
        cmd.build();
        count_help_lines(&mut cmd, sub)
    };

    let n = TISH_BANNER_LINES.len();

    // 1. First banner frame + manual prefix + full help – all visible immediately.
    {
        let mut out = io::stdout().lock();
        write_tish_banner_frame(&mut out, 1.0, 0);
        let _ = writeln!(out); // blank separator  (row n+1)
        // ── Manual prefix (MAIN_PREFIX_LINES = 4 lines) ──────────────────
        let _ = writeln!(out, "{H_PURPLE}Tish{H_RESET} {H_GREY}(version {VERSION}){H_RESET}");
        let _ = writeln!(out, "Minimal TS/JS-ish language");
        let _ = writeln!(out, "{H_PINK}https://tishlang.com{H_RESET}");
        let _ = writeln!(out); // blank before Usage
        let _ = out.flush();
    }
    {
        let mut cmd = build_command();
        cmd.build();
        print_help_to_stdout(&mut cmd, sub);
        let _ = io::stdout().flush();
    }

    // 2. Jump cursor back to banner top and cycle colors.
    // Total rows above cursor: n + 1 (sep) + MAIN_PREFIX_LINES + h (clap)
    {
        let mut out = io::stdout().lock();
        let _ = write!(out, "\x1b[{}A", n + 1 + MAIN_PREFIX_LINES + h);
        let _ = out.flush();

        let frames = BANNER_CYCLE_FRAMES;
        for f in 0..frames {
            write_tish_banner_frame(&mut out, 1.0, f);
            if f < frames - 1 {
                let _ = write!(out, "\x1b[{}A", n);
            }
            let _ = out.flush();
            thread::sleep(Duration::from_millis(BANNER_FRAME_MS));
        }

        // After last frame cursor is at row n; skip sep + prefix + clap rows.
        let _ = write!(out, "\x1b[{}B", 1 + MAIN_PREFIX_LINES + h);
        let _ = writeln!(out);
        let _ = out.flush();
    }
}

/// Whether argv will cause clap to print help (top-level or subcommand).
pub fn argv_requests_help(argv: &[String]) -> bool {
    argv.iter().any(|a| a == "--help" || a == "-h")
        || matches!(argv.get(1).map(String::as_str), Some("help"))
}

/// Build a bold true-color `Style` from the brand palette.
fn rgb_bold(r: u8, g: u8, b: u8) -> Style {
    Style::new().fg_color(Some(Color::Rgb(RgbColor(r, g, b)))) | Effects::BOLD
}

/// Help colors using the brand palette.
/// Orange → section headers / usage.  Teal → literals (commands, flags).  Yellow → placeholders.
pub fn cargo_help_styles() -> Styles {
    Styles::styled()
        .header(rgb_bold(255, 159,  64))      // Orange  – "Commands:", "Options:", "Usage:"
        .usage(rgb_bold(255, 159,  64))       // Orange
        .literal(rgb_bold( 48, 209, 188))     // Teal    – run, repl, --help, -V …
        .placeholder(rgb_bold(255, 213,  64)) // Yellow  – <FILE>, <NAME>, …
        .error(rgb_bold(255,  55, 148))       // Pink    – error messages
        .valid(rgb_bold( 52, 199,  89))       // Green   – valid values
        .invalid(rgb_bold(255,  55, 148))     // Pink    – invalid values
}

/// Returns the colored `after_help` text for the top-level `tish --help`.
/// Colors are emitted only when stdout is a TTY.
pub fn cli_after_help() -> String {
    let (oh, t, r) = if io::stdout().is_terminal() {
        ("\x1b[1;38;2;255;159;64m", "\x1b[1;38;2;48;209;188m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    format!(
        "\
{oh}Environment variables:{r}
  {t}TISH_NO_OPTIMIZE=1{r}
          Disable AST and bytecode optimizations for run/build

See {t}tish run --help{r} and {t}tish build --help{r} for backend and feature options."
    )
}

fn capabilities_section(oh: &str, t: &str, r: &str) -> String {
    format!(
        "\
{oh}Backends{r} (--backend):
  {t}vm{r}
          Bytecode VM (default)
  {t}interp{r}
          Tree-walking interpreter

{oh}Capabilities{r} (--feature, repeatable; comma-separated values are split):
  {t}http{r}
          Network: fetch, serve, Promise / timers (native async)
  {t}fs{r}
          Filesystem: readFile, writeFile, fileExists, isDir, readDir, mkdir
  {t}process{r}
          process.exit, cwd, exec, argv, env
  {t}regex{r}
          RegExp
  {t}ws{r}
          WebSocket client / server
  {t}full{r}
          All of the above (http, fs, process, regex, ws)

Omit --feature to use every capability linked into this binary."
    )
}

/// Returns the colored `after_help` for `tish run --help`.
pub fn run_after_help() -> String {
    let (oh, t, r) = if io::stdout().is_terminal() {
        ("\x1b[1;38;2;255;159;64m", "\x1b[1;38;2;48;209;188m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    capabilities_section(oh, t, r)
}

/// Returns the colored `after_help` for `tish repl --help`.
pub fn repl_after_help() -> String {
    let (oh, t, r) = if io::stdout().is_terminal() {
        ("\x1b[1;38;2;255;159;64m", "\x1b[1;38;2;48;209;188m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    capabilities_section(oh, t, r)
}

/// Returns the colored `after_long_help` for `tish build --help`.
pub fn build_after_help() -> String {
    let (oh, t, r) = if io::stdout().is_terminal() {
        ("\x1b[1;38;2;255;159;64m", "\x1b[1;38;2;48;209;188m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    format!(
        "\
{oh}Build targets{r} (--target, default: native):
  {t}native{r}
          Native executable (see --native-backend)
  {t}js{r}
          JavaScript bundle
  {t}wasm{r}
          WebAssembly (.tish project; .js source supported on some paths)
  {t}wasi{r}
          WASI WebAssembly

{oh}Native backends{r} (--native-backend, only with --target native, default: rust):
  {t}rust{r}
          Emit Rust + link tishlang_runtime via cargo
  {t}cranelift{r}
          Embedded bytecode + Cranelift/VM runtime binary
  {t}llvm{r}
          Embedded bytecode + LLVM/clang link path

{oh}Capabilities{r} (--feature, repeatable; comma-separated values are split):
  {t}http{r}
          Network: fetch, serve, Promise / timers (native async)
  {t}fs{r}
          Filesystem: readFile, writeFile, fileExists, isDir, readDir, mkdir
  {t}process{r}
          process.exit, cwd, exec, argv, env
  {t}regex{r}
          RegExp
  {t}ws{r}
          WebSocket client / server
  {t}full{r}
          All of the above (http, fs, process, regex, ws)

Omit --feature to use every capability linked into this binary.
Build `tish` with matching Cargo features (e.g. cargo build -p tishlang --features full)."
    )
}

#[derive(Parser)]
#[command(name = "tish")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(styles = cargo_help_styles())]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Parser)]
pub(crate) struct RunArgs {
    /// Path to a `.tish` file, or `-` to read the program from stdin (like `node -`).
    #[arg(required = true, allow_hyphen_values = true, value_name = "FILE", help_heading = "Arguments")]
    pub file: String,
    /// `vm` or `interp` (see `tish --help` for capabilities / `--feature`).
    #[arg(long, default_value = "vm", value_name = "NAME", help_heading = "Options")]
    pub backend: String,
    /// Subset of capabilities (see `tish --help` for the full list).
    #[arg(
        long = "feature",
        value_name = "NAME",
        action = clap::ArgAction::Append,
        help_heading = "Options"
    )]
    pub features: Vec<String>,
    /// Disable AST and bytecode optimizations (for debugging).
    #[arg(long, help_heading = "Options")]
    pub no_optimize: bool,
}

#[derive(Parser)]
pub(crate) struct ReplArgs {
    /// `vm` or `interp` (see `tish --help`).
    #[arg(long, default_value = "vm", value_name = "NAME", help_heading = "Options")]
    pub backend: String,
    /// Subset of capabilities (see `tish --help` for the full list).
    #[arg(
        long = "feature",
        value_name = "NAME",
        action = clap::ArgAction::Append,
        help_heading = "Options"
    )]
    pub features: Vec<String>,
    #[arg(long, help_heading = "Options")]
    pub no_optimize: bool,
}

#[derive(Parser)]
pub(crate) struct BuildArgs {
    #[arg(
        short,
        long,
        default_value = "tish_out",
        value_name = "PATH",
        help_heading = "Options"
    )]
    pub output: String,
    /// `native`, `js`, `wasm`, or `wasi` (see long help below).
    #[arg(long, default_value = "native", value_name = "TARGET", help_heading = "Options")]
    pub target: String,
    /// `rust`, `cranelift`, or `llvm` (only for `--target native`).
    #[arg(long, default_value = "rust", value_name = "BACKEND", help_heading = "Options")]
    pub native_backend: String,
    /// Capability subset for native output (see long help below).
    #[arg(
        long = "feature",
        value_name = "NAME",
        action = clap::ArgAction::Append,
        help_heading = "Options"
    )]
    pub features: Vec<String>,
    #[arg(long, help_heading = "Options")]
    pub no_optimize: bool,
    /// Entry `.tish` file (or `.js` where supported).
    #[arg(required = true, value_name = "FILE", help_heading = "Arguments")]
    pub file: String,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Run a Tish file (interpret)
    Run(RunArgs),
    /// Interactive REPL
    Repl(ReplArgs),
    /// Build native binary, wasm, wasi, or JavaScript output
    Build(BuildArgs),
    /// Parse and dump AST
    #[command(name = "dump-ast")]
    DumpAst {
        #[arg(required = true, value_name = "FILE", help_heading = "Arguments")]
        file: String,
    },
}
