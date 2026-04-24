//! Process-level prefork for the `tish:http` server.
//!
//! ## Why
//!
//! Tish's `Value` type is reference-counted with `Rc`/`RefCell` and therefore
//! `!Send`. Serving HTTP in parallel across CPU cores with the existing VM
//! would require either
//!
//!   1. a wholesale `Rc → Arc` conversion across every Tish crate, or
//!   2. spinning up independent VM instances that never share a `Value`.
//!
//! Option 1 taxes every single-threaded Tish program with atomic ref-count
//! overhead forever. Option 2 is what this file implements, via the classic
//! UNIX **prefork** pattern:
//!
//! * The parent process (worker 0) `fork`s — actually `spawn`s a new
//!   `std::process::Command` pointing at the current executable — once per
//!   extra core. Each child re-executes the entire Tish program in its own
//!   address space.
//! * All processes (parent + children) bind the same `port` with
//!   `SO_REUSEPORT`; the kernel hashes incoming connections across them.
//! * Each process runs a *single-threaded* accept + dispatch loop, so the
//!   Tish VM stays single-threaded and `Value` stays `Rc`-backed.
//!
//! ## Why this is the right default
//!
//! * **nginx, gunicorn, unicorn, puma (cluster), and phpfpm all ship this
//!   model.** It's the battle-tested way to extract N-core throughput from a
//!   single-threaded scripting runtime.
//! * Zero Tish-language semantic changes: users write `serve(port, handler)`
//!   exactly as before and get N-core scaling for free.
//! * Every process has a fresh DB connection pool, a fresh cache, a fresh
//!   whatever. No cache invalidation, no shared mutable state, no data races.
//! * Crash isolation: if one worker panics the others keep serving.
//!
//! ## Cost
//!
//! * Each worker re-runs top-level initialization (module imports, constant
//!   folding, static route registration, cache warmup, ...). For typical
//!   apps this is milliseconds and happens once at startup, in parallel. For
//!   apps that preload hundreds of MB of in-process state (e.g. the TFB
//!   `warmupWorldCache` that keeps 10 000 rows in RAM), the memory multiplier
//!   is N×. Users who can't afford the memory set `TISH_HTTP_WORKERS=1`.
//!
//! ## Control surface
//!
//! | env var             | default                | effect                               |
//! |---------------------|------------------------|--------------------------------------|
//! | `TISH_HTTP_WORKERS` | `available_parallelism`| number of worker processes           |
//! | `TISH_HTTP_PREFORK` | `1` (on)               | set to `0` to force single-process   |
//! | `TISH_WORKER_ID`    | unset on parent        | set on children by the parent        |
//!
//! Children are launched with `TISH_WORKER_ID={1..N-1}` and
//! `TISH_HTTP_PREFORK=child`. The serve() runtime checks these before
//! deciding whether to fork again, preventing runaway forking.

use std::io;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Role of the current process in a prefork group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreforkRole {
    /// The parent — owns the child PIDs, handles signals, re-execs nothing.
    Parent,
    /// A child spawned by the parent. Never forks again.
    Child(usize),
    /// Prefork disabled (single-process mode).
    Single,
}

/// Inspect the environment to decide which role this process plays.
pub fn role_from_env() -> PreforkRole {
    let is_child = std::env::var("TISH_HTTP_PREFORK")
        .map(|v| v.eq_ignore_ascii_case("child"))
        .unwrap_or(false);
    if is_child {
        let id = std::env::var("TISH_WORKER_ID")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);
        return PreforkRole::Child(id);
    }
    let disabled = std::env::var("TISH_HTTP_PREFORK")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        .unwrap_or(false);
    if disabled {
        PreforkRole::Single
    } else {
        PreforkRole::Parent
    }
}

/// Spawn `n - 1` child processes (current worker is worker 0). Each child
/// inherits stdio and gets `TISH_WORKER_ID={1..n-1}` +
/// `TISH_HTTP_PREFORK=child` so it doesn't recurse.
///
/// Returns the child handles so the parent can reap / signal them.
pub fn spawn_children(n: usize) -> io::Result<Vec<Child>> {
    if n <= 1 {
        return Ok(Vec::new());
    }
    let exe = std::env::current_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut out = Vec::with_capacity(n - 1);
    for i in 1..n {
        let mut cmd = Command::new(&exe);
        cmd.args(&args);
        cmd.env("TISH_WORKER_ID", i.to_string());
        cmd.env("TISH_HTTP_PREFORK", "child");
        // Children inherit the shared cache of 1 thread so they don't recurse
        // into SO_REUSEPORT multi-listener logic. The parent keeps the same.
        cmd.env("TISH_HTTP_WORKERS", "1");
        // Inherit stdout/stderr: child logs stream into the same terminal.
        let child = cmd.spawn()?;
        out.push(child);
    }
    Ok(out)
}

/// Install a Ctrl-C / SIGTERM handler on the parent that propagates to all
/// children. Safe to call multiple times; the handler is stored in a
/// process-wide slot.
///
/// Returns a shared stop flag that callers can poll from their accept loop.
pub fn install_parent_signal_handler(children: Vec<Child>) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let pids: Vec<u32> = children.iter().map(|c| c.id()).collect();
    install_shutdown_handler(Arc::clone(&stop), pids);

    // Reap children in the background so they don't zombify when the user
    // ^C's or when a child dies on its own.
    std::thread::Builder::new()
        .name("tish-prefork-reaper".into())
        .spawn(move || {
            for mut child in children {
                let _ = child.wait();
            }
        })
        .ok();
    stop
}

#[cfg(unix)]
fn install_shutdown_handler(stop: Arc<AtomicBool>, pids: Vec<u32>) {
    // Store state in process-global statics so an `extern "C"` fn can reach
    // them from inside a signal handler. This is the usual pattern for
    // libc::signal callbacks — setting a flag + waking up listeners is the
    // only async-signal-safe work we do here.
    use std::sync::OnceLock;
    static STOP_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    static CHILD_PIDS: OnceLock<Vec<u32>> = OnceLock::new();

    let _ = STOP_FLAG.set(stop);
    let _ = CHILD_PIDS.set(pids);

    extern "C" fn on_signal(sig: libc::c_int) {
        if let Some(flag) = STOP_FLAG.get() {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(pids) = CHILD_PIDS.get() {
            for pid in pids {
                unsafe {
                    libc::kill(*pid as libc::pid_t, libc::SIGTERM);
                }
            }
        }
        // Re-raise with default disposition so the parent actually exits.
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }

    let h = on_signal as *const () as libc::sighandler_t;
    unsafe {
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
    }
}

#[cfg(not(unix))]
fn install_shutdown_handler(_stop: Arc<AtomicBool>, _pids: Vec<u32>) {
    // TODO: SetConsoleCtrlHandler on Windows.
}
