//! Interactive terminal I/O for Tish (issue #101), behind the `tty` feature.
//!
//! Exposes raw mode, the alternate screen, terminal size, and key/resize **events** via
//! `crossterm`, so Tish programs can build interactive TUIs (menus, forms, live keyboard
//! navigation). Imported as `import { … } from 'tish:tty'`.
//!
//! The Value-agnostic core (`size`, `is_tty`, `set_raw_mode`, `read_event`, …) returns plain
//! Rust data so every backend — the bytecode VM (via the `tty_*` wrappers here) and the
//! tree-walk interpreter (whose `Value` is a distinct type) — can build its own `Value` from
//! the same logic. Errors surface as `null`/`false` rather than panicking.

use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::time::Duration;

use tishlang_core::{ObjectMap, Value};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

/// A terminal event delivered by [`read_event`]. Plain data so each backend maps it to its
/// own `Value` object.
pub enum TtyEvent {
    Key {
        key: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    /// Mouse / focus / paste — reported generically so an input loop can ignore it.
    Other,
}

// ── Value-agnostic core (shared by every backend) ───────────────────────────────────────

/// Terminal `(cols, rows)`, or `None` if not connected to a terminal.
pub fn size() -> Option<(u16, u16)> {
    terminal::size().ok()
}

/// Whether stdin **and** stdout are connected to a terminal.
pub fn is_tty() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Enter (cbreak) or leave raw mode. Returns `true` on success.
pub fn set_raw_mode(enabled: bool) -> bool {
    if enabled {
        terminal::enable_raw_mode().is_ok()
    } else {
        terminal::disable_raw_mode().is_ok()
    }
}

/// Switch to / from the alternate screen buffer (full-screen apps).
pub fn enter_alt_screen() -> bool {
    let mut out = std::io::stdout();
    let ok = crossterm::execute!(out, terminal::EnterAlternateScreen).is_ok();
    let _ = out.flush();
    ok
}
pub fn leave_alt_screen() -> bool {
    let mut out = std::io::stdout();
    let ok = crossterm::execute!(out, terminal::LeaveAlternateScreen).is_ok();
    let _ = out.flush();
    ok
}

/// Read the next terminal event. `timeout_ms = None` blocks; `Some(ms)` polls for `ms`
/// milliseconds (0 = non-blocking) and returns `None` on timeout. Key-release events
/// (Windows) are skipped, yielding `None`.
pub fn read_event(timeout_ms: Option<u64>) -> Option<TtyEvent> {
    if let Some(ms) = timeout_ms {
        match event::poll(Duration::from_millis(ms)) {
            Ok(true) => {}
            _ => return None,
        }
    }
    match event::read().ok()? {
        Event::Key(k) => {
            if k.kind == KeyEventKind::Release {
                return None;
            }
            Some(TtyEvent::Key {
                key: key_code_name(k.code),
                ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
                alt: k.modifiers.contains(KeyModifiers::ALT),
                shift: k.modifiers.contains(KeyModifiers::SHIFT),
            })
        }
        Event::Resize(cols, rows) => Some(TtyEvent::Resize { cols, rows }),
        _ => Some(TtyEvent::Other),
    }
}

/// Read one line of **cooked** input from stdin (line mode), with the trailing newline
/// stripped. Returns `None` at EOF. For raw key-by-key input use [`read_event`].
pub fn read_line() -> Option<String> {
    use std::io::BufRead;
    let mut s = String::new();
    match std::io::stdin().lock().read_line(&mut s) {
        Ok(0) => None,
        Ok(_) => {
            while s.ends_with('\n') || s.ends_with('\r') {
                s.pop();
            }
            Some(s)
        }
        Err(_) => None,
    }
}

/// Normalize a crossterm key code to a stable JS-friendly name (`"a"`, `"Enter"`, `"Up"`,
/// `"Esc"`, `"F1"`, …).
fn key_code_name(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Null => "Null".into(),
        other => format!("{other:?}"),
    }
}

// ── core::Value wrappers for the bytecode VM / native runtime ────────────────────────────

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = ObjectMap::default();
    for (k, v) in pairs {
        m.insert(Arc::from(k), v);
    }
    Value::object(m)
}

/// `size()` → `{ cols, rows }` or `null`.
pub fn tty_size(_args: &[Value]) -> Value {
    match size() {
        Some((cols, rows)) => obj(vec![
            ("cols", Value::Number(cols as f64)),
            ("rows", Value::Number(rows as f64)),
        ]),
        None => Value::Null,
    }
}

/// `isTTY()` → bool.
pub fn tty_is_tty(_args: &[Value]) -> Value {
    Value::Bool(is_tty())
}

/// `setRawMode(enabled)` → bool (success).
pub fn tty_set_raw_mode(args: &[Value]) -> Value {
    Value::Bool(set_raw_mode(args.first().map(|v| v.is_truthy()).unwrap_or(false)))
}

/// `enterAltScreen()` / `leaveAltScreen()` → bool.
pub fn tty_enter_alt_screen(_args: &[Value]) -> Value {
    Value::Bool(enter_alt_screen())
}
pub fn tty_leave_alt_screen(_args: &[Value]) -> Value {
    Value::Bool(leave_alt_screen())
}

/// `readLine()` → one line of cooked stdin (no trailing newline), or `null` at EOF.
pub fn tty_read_line(_args: &[Value]) -> Value {
    match read_line() {
        Some(s) => Value::String(s.into()),
        None => Value::Null,
    }
}

/// `read(timeoutMs?)` → an event object (`{ type, … }`) or `null`.
pub fn tty_read(args: &[Value]) -> Value {
    let timeout = match args.first() {
        Some(Value::Number(ms)) => Some(ms.max(0.0) as u64),
        _ => None,
    };
    match read_event(timeout) {
        Some(TtyEvent::Key {
            key,
            ctrl,
            alt,
            shift,
        }) => obj(vec![
            ("type", Value::String("key".into())),
            ("key", Value::String(key.into())),
            ("ctrl", Value::Bool(ctrl)),
            ("alt", Value::Bool(alt)),
            ("shift", Value::Bool(shift)),
        ]),
        Some(TtyEvent::Resize { cols, rows }) => obj(vec![
            ("type", Value::String("resize".into())),
            ("cols", Value::Number(cols as f64)),
            ("rows", Value::Number(rows as f64)),
        ]),
        Some(TtyEvent::Other) => obj(vec![("type", Value::String("other".into()))]),
        None => Value::Null,
    }
}
