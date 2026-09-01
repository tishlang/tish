//! Shared reader-thread buffer + UTF-8-boundary drain for the three byte-stream modules
//! (`process_spawn`, `net`, `pty`). These used to carry three byte-identical copies of the
//! same append loop and drain guard; this module is the single implementation.
//!
//! Semantics (identical for every consumer):
//!
//! - **Bounded buffer with backpressure.** Each stream buffers at most `TISH_STREAM_MAX_BUF`
//!   bytes (default 8 MiB; the env override mirrors `TISH_WS_MAX_CONNS` in `ws.rs`). When the
//!   buffer is full the reader thread *parks* on the condvar instead of appending, so the OS
//!   pipe/socket fills and the producer blocks on `write(2)` — restoring exactly the kernel
//!   backpressure a greedy reader thread otherwise defeats. Draining (or retiring) the buffer
//!   wakes the reader. The cap is approximate up to one read chunk (8 KiB).
//! - **UTF-8 drain that always makes progress.** `drain_stream` returns only complete UTF-8:
//!   a genuinely *incomplete* trailing multi-byte sequence (`Utf8Error::error_len() == None`,
//!   at most 3 bytes) is held for the next call, but *invalid* bytes
//!   (`error_len() == Some(_)`) can never become valid no matter what arrives later — they are
//!   drained and lossy-converted to U+FFFD so a single bad byte cannot wedge the stream.
//! - **Capacity release.** After a drain the Vec is shrunk when it retains far more capacity
//!   than it holds, so a one-off burst does not pin its high-water allocation forever.
//! - **Retirement.** `retire_buf` marks the buffer dead and wakes a parked reader so the
//!   thread exits (used by kill/close paths whose fd shutdown alone cannot wake a parked —
//!   as opposed to read-blocked — reader).
//!
//! Contract surfaced to tish code (unchanged): `""` = live but nothing readable yet,
//! `null` = EOF (drained) or unknown id.

use std::io::Read;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use tishlang_core::Value;

/// Output accumulated by a reader thread, drained by the module's read fn.
pub(crate) struct StreamBuf {
    pub(crate) data: Vec<u8>,
    pub(crate) eof: bool,
    /// Set by `retire_buf` when the owning session/connection is being torn down; a parked
    /// reader wakes, sees it, and exits.
    pub(crate) retired: bool,
}

pub(crate) type SharedBuf = Arc<(Mutex<StreamBuf>, Condvar)>;

pub(crate) fn new_buf() -> SharedBuf {
    Arc::new((
        Mutex::new(StreamBuf {
            data: Vec::new(),
            eof: false,
            retired: false,
        }),
        Condvar::new(),
    ))
}

/// Max bytes buffered per stream before the reader thread parks (drain to resume).
/// Override with `TISH_STREAM_MAX_BUF`; default 8 MiB.
pub(crate) fn max_stream_buf() -> usize {
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("TISH_STREAM_MAX_BUF")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&v: &usize| v > 0)
            .unwrap_or(8 << 20)
    })
}

/// Reader thread: block on the fd, append bytes (parking at the cap), wake any waiting
/// reader; mark EOF on 0/err.
pub(crate) fn spawn_reader<R: Read + Send + 'static>(r: R, buf: SharedBuf) {
    let cap = max_stream_buf();
    std::thread::spawn(move || reader_loop(r, buf, cap));
}

/// The reader-thread body, cap-injectable for tests.
pub(crate) fn reader_loop<R: Read>(mut r: R, buf: SharedBuf, cap: usize) {
    let mut tmp = [0u8; 8192];
    loop {
        // Backpressure: while the buffer is at capacity, park until a drain (or retirement)
        // makes room. The producer then blocks on the full OS pipe/socket instead of growing
        // our heap.
        {
            let (lock, cv) = &*buf;
            let mut b = match lock.lock() {
                Ok(b) => b,
                Err(_) => return,
            };
            while b.data.len() >= cap && !b.retired {
                b = match cv.wait(b) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
            if b.retired {
                return;
            }
        }
        match r.read(&mut tmp) {
            Ok(0) => {
                mark_eof(&buf);
                return;
            }
            Ok(n) => {
                let (lock, cv) = &*buf;
                if let Ok(mut b) = lock.lock() {
                    if b.retired {
                        return;
                    }
                    b.data.extend_from_slice(&tmp[..n]);
                }
                cv.notify_all();
            }
            Err(_) => {
                mark_eof(&buf);
                return;
            }
        }
    }
}

fn mark_eof(buf: &SharedBuf) {
    let (lock, cv) = &**buf;
    if let Ok(mut b) = lock.lock() {
        b.eof = true;
    }
    cv.notify_all();
}

/// Mark a buffer dead and wake its (possibly parked) reader thread so it exits. Call from
/// kill/close paths; harmless if the reader already exited.
pub(crate) fn retire_buf(buf: &SharedBuf) {
    let (lock, cv) = &**buf;
    if let Ok(mut b) = lock.lock() {
        b.retired = true;
    }
    cv.notify_all();
}

/// Whether the stream is fully consumed: reader saw EOF and everything was drained.
/// (`eof` also proves the reader thread has exited.)
pub(crate) fn is_drained(buf: &SharedBuf) -> bool {
    let (lock, _) = &**buf;
    lock.lock()
        .map(|b| b.eof && b.data.is_empty())
        .unwrap_or(false)
}

/// How many leading bytes can be drained right now. Everything is drainable except a
/// genuinely incomplete trailing multi-byte sequence (`error_len() == None`), which is held
/// for the next read — unless the stream is at EOF (nothing more will arrive) or the held
/// tail is impossibly long for a UTF-8 prefix (>= 4 bytes; cannot happen per the std
/// contract, kept as a safety valve so the stream can never wedge).
fn drainable_len(data: &[u8], eof: bool) -> usize {
    if eof {
        return data.len();
    }
    let mut i = 0;
    loop {
        match std::str::from_utf8(&data[i..]) {
            Ok(_) => return data.len(),
            Err(e) => match e.error_len() {
                // Invalid bytes: will never become valid — drainable (lossy-converted).
                Some(n) => i += e.valid_up_to() + n,
                // Incomplete tail: hold it (it is 1..=3 bytes) for the next read.
                None => {
                    let held_from = i + e.valid_up_to();
                    if data.len() - held_from >= 4 {
                        return data.len();
                    }
                    return held_from;
                }
            },
        }
    }
}

/// After a drain, release a burst's high-water capacity: if the Vec retains more than 4x
/// what it now holds (beyond a small keep-floor), shrink it back down.
fn shrink_after_drain(data: &mut Vec<u8>) {
    const KEEP: usize = 16 * 1024;
    if data.capacity() > KEEP && data.capacity() > data.len().saturating_mul(4) {
        data.shrink_to(data.len().max(KEEP));
    }
}

/// Drain a stream buffer as a string (possibly `""` within the timeout), or `null` at EOF.
/// Invalid bytes are replaced with U+FFFD; only an incomplete trailing multi-byte sequence
/// is held for the next call (no mojibake, no permanent stall).
pub(crate) fn drain_stream(buf: &SharedBuf, timeout_ms: u64) -> Value {
    let (lock, cv) = &**buf;
    let mut b = match lock.lock() {
        Ok(b) => b,
        Err(_) => return Value::Null,
    };
    if b.data.is_empty() && !b.eof && timeout_ms > 0 {
        let res = cv.wait_timeout_while(b, Duration::from_millis(timeout_ms), |b| {
            b.data.is_empty() && !b.eof
        });
        b = match res {
            Ok((g, _)) => g,
            Err(e) => e.into_inner().0,
        };
    }
    if b.data.is_empty() {
        if b.eof {
            return Value::Null;
        }
        return Value::String("".into());
    }
    let take = drainable_len(&b.data, b.eof);
    if take == 0 {
        // Only an incomplete multi-byte tail is buffered; wait for the rest.
        return Value::String("".into());
    }
    let out: Vec<u8> = b.data.drain(..take).collect();
    shrink_after_drain(&mut b.data);
    // Wake a reader parked at the cap: there is room again.
    cv.notify_all();
    drop(b);
    let s = match String::from_utf8(out) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    Value::String(s.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_with(data: &[u8], eof: bool) -> SharedBuf {
        let b = new_buf();
        {
            let mut g = b.0.lock().unwrap();
            g.data.extend_from_slice(data);
            g.eof = eof;
        }
        b
    }

    fn drain_str(buf: &SharedBuf) -> String {
        match drain_stream(buf, 0) {
            Value::String(s) => s.to_string(),
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn invalid_byte_mid_stream_is_replaced_not_wedged() {
        // Pre-fix: valid_up_to()==0 for a leading invalid byte returned "" forever.
        let b = buf_with(b"\xffhello", false);
        assert_eq!(drain_str(&b), "\u{fffd}hello");
        assert!(b.0.lock().unwrap().data.is_empty());
    }

    #[test]
    fn invalid_bytes_between_valid_text_drain_through() {
        let b = buf_with(b"ok\xff\xfemore", false);
        assert_eq!(drain_str(&b), "ok\u{fffd}\u{fffd}more");
    }

    #[test]
    fn incomplete_tail_is_held_then_completed() {
        // "ab" + first 2 bytes of U+20AC (€ = E2 82 AC).
        let b = buf_with(b"ab\xe2\x82", false);
        assert_eq!(drain_str(&b), "ab");
        assert_eq!(b.0.lock().unwrap().data.len(), 2);
        // Tail alone: still held, drain returns "" (live).
        assert_eq!(drain_str(&b), "");
        // The final byte arrives: the whole code point drains.
        b.0.lock().unwrap().data.push(0xac);
        assert_eq!(drain_str(&b), "\u{20ac}");
    }

    #[test]
    fn incomplete_tail_flushes_lossily_at_eof() {
        let b = buf_with(b"\xe2\x82", true);
        assert_eq!(drain_str(&b), "\u{fffd}");
        assert!(matches!(drain_stream(&b, 0), Value::Null));
    }

    #[test]
    fn empty_buf_semantics_preserved() {
        let live = buf_with(b"", false);
        assert_eq!(drain_str(&live), "");
        let done = buf_with(b"", true);
        assert!(matches!(drain_stream(&done, 0), Value::Null));
    }

    #[test]
    fn reader_parks_at_cap_and_resumes_after_drain() {
        // io::repeat never blocks, so an uncapped reader would grow without bound instantly.
        const CAP: usize = 16 * 1024;
        let buf = new_buf();
        let b2 = buf.clone();
        let handle = std::thread::spawn(move || reader_loop(std::io::repeat(b'x'), b2, CAP));

        // Fills to the cap, then parks. Cap is approximate up to one 8 KiB chunk.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let len = buf.0.lock().unwrap().data.len();
            if len >= CAP {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reader never reached cap"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        let len = buf.0.lock().unwrap().data.len();
        assert!(len <= CAP + 8192, "buffer exceeded cap: {}", len);

        // Draining makes room; the reader wakes and refills — still bounded.
        let drained = drain_str(&buf);
        assert!(!drained.is_empty());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let len = buf.0.lock().unwrap().data.len();
            if len >= CAP {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reader did not resume after drain"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let len = buf.0.lock().unwrap().data.len();
        assert!(
            len <= CAP + 8192,
            "buffer exceeded cap after refill: {}",
            len
        );

        // Retirement wakes the parked reader so the thread exits (kill/close path).
        retire_buf(&buf);
        handle
            .join()
            .expect("reader thread did not exit after retire");
    }

    #[test]
    fn drain_shrinks_high_water_capacity() {
        let b = new_buf();
        {
            let mut g = b.0.lock().unwrap();
            g.data = Vec::with_capacity(4 << 20);
            g.data.extend_from_slice(&vec![b'a'; 4 << 20]);
        }
        let s = drain_str(&b);
        assert_eq!(s.len(), 4 << 20);
        let g = b.0.lock().unwrap();
        assert!(
            g.data.capacity() <= 64 * 1024,
            "capacity not released: {}",
            g.data.capacity()
        );
    }
}
