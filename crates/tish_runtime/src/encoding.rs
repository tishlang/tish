//! Base64 encoding for Tish (`tish:encoding`), behind the `encoding` feature.
//!
//!   import { base64Encode, base64Decode, base64UrlEncode, base64UrlDecode } from 'tish:encoding'
//!
//!   - `base64Encode(bytesOrString) -> string`       (standard alphabet, padded)
//!   - `base64Decode(string) -> number[] | null`     (bytes 0-255; null on invalid input)
//!   - `base64UrlEncode(bytesOrString) -> string`    (URL-safe alphabet, NO padding — PKCE etc.)
//!   - `base64UrlDecode(string) -> number[] | null`
//!
//! Bytes are the same representation `tish:fs` uses: an array of Numbers 0-255 (`readFileBytes`
//! returns this; `writeFile` accepts it). A string argument is encoded as its UTF-8 bytes, so
//! `base64Encode("hi")` works without a byte round-trip.

use base64::Engine;
use tishlang_core::{Value, VmRef};

/// Coerce arg 0 to bytes: a byte array (Numbers 0-255) or a string's UTF-8 bytes.
fn arg_bytes(args: &[Value]) -> Option<Vec<u8>> {
    match args.first() {
        Some(Value::Array(a)) => Some(
            a.borrow()
                .iter()
                .map(|v| if let Value::Number(n) = v { *n as u8 } else { 0 })
                .collect(),
        ),
        Some(Value::String(s)) => Some(s.as_bytes().to_vec()),
        _ => None,
    }
}

fn bytes_to_value(bytes: Vec<u8>) -> Value {
    Value::Array(VmRef::new(
        bytes.into_iter().map(|b| Value::Number(b as f64)).collect(),
    ))
}

pub fn base64_encode(args: &[Value]) -> Value {
    match arg_bytes(args) {
        Some(b) => Value::String(base64::engine::general_purpose::STANDARD.encode(&b).into()),
        None => Value::Null,
    }
}

pub fn base64_decode(args: &[Value]) -> Value {
    let s = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => return Value::Null,
    };
    match base64::engine::general_purpose::STANDARD.decode(s.as_bytes()) {
        Ok(b) => bytes_to_value(b),
        Err(_) => Value::Null,
    }
}

pub fn base64_url_encode(args: &[Value]) -> Value {
    match arg_bytes(args) {
        Some(b) => Value::String(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(&b)
                .into(),
        ),
        None => Value::Null,
    }
}

pub fn base64_url_decode(args: &[Value]) -> Value {
    let s = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => return Value::Null,
    };
    match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.as_bytes()) {
        Ok(b) => bytes_to_value(b),
        Err(_) => Value::Null,
    }
}

/// `utf8Decode(byteArray) -> string` — decode bytes as UTF-8 (lossy: invalid sequences become the
/// replacement char). The counterpart to `readFileBytes` / a streaming `reader.read()` chunk, whose
/// `value` is a byte array. Split streamed bytes on a newline BEFORE decoding a line so a multi-byte
/// sequence is never cut mid-decode (a `\n`=0x0A byte never occurs inside a UTF-8 multi-byte char).
pub fn utf8_decode(args: &[Value]) -> Value {
    match arg_bytes(args) {
        Some(b) => Value::String(String::from_utf8_lossy(&b).into_owned().into()),
        None => Value::Null,
    }
}

/// `utf8Complete(byteArray) -> number` — how many leading bytes end on a whole character. A streamed
/// chunk (`reader.read()`) can stop inside a multi-byte sequence, and decoding it then corrupts that
/// character; decode `bytes.slice(0, n)` and carry the rest into the next chunk instead. Bytes that can
/// never become a valid character count as complete (they decode to the replacement character rather
/// than being held back forever).
pub fn utf8_complete(args: &[Value]) -> Value {
    match arg_bytes(args) {
        Some(b) => Value::Number(utf8_complete_len(&b) as f64),
        None => Value::Null,
    }
}

fn utf8_complete_len(b: &[u8]) -> usize {
    let n = b.len();
    // A UTF-8 character is at most 4 bytes, so only the last 3 can belong to an unfinished one.
    let mut i = n;
    while i > n.saturating_sub(3) {
        i -= 1;
        let c = b[i];
        if c & 0xC0 == 0x80 {
            continue; // continuation byte: keep looking for the lead byte
        }
        let len = match c {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 1, // never valid as a lead byte: complete as it is
        };
        return if n - i < len { i } else { n };
    }
    n
}

/// `utf8Encode(string) -> number[]` — the string's UTF-8 bytes as a byte array. (A byte-array
/// argument is passed through, so `utf8Encode(utf8Decode(x))` is a no-op round trip.)
pub fn utf8_encode(args: &[Value]) -> Value {
    match arg_bytes(args) {
        Some(b) => bytes_to_value(b),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_bytes(v: Value) -> Vec<u8> {
        let Value::Array(a) = v else { panic!("not an array") };
        let g = a.borrow();
        let out: Vec<u8> = g
            .iter()
            .map(|x| if let Value::Number(n) = x { *n as u8 } else { 0 })
            .collect();
        out
    }

    fn complete(bytes: &[u8]) -> usize {
        let v = Value::Array(VmRef::new(bytes.iter().map(|b| Value::Number(*b as f64)).collect()));
        let Value::Number(n) = utf8_complete(&[v]) else { panic!("not a number") };
        n as usize
    }

    #[test]
    fn utf8_complete_holds_back_an_unfinished_character() {
        let text = "café ✓ 🙂".as_bytes();
        assert_eq!(complete(text), text.len());
        // Cut inside "é" (2 bytes), "✓" (3) and "🙂" (4): the lead byte and what follows wait.
        assert_eq!(complete(&text[..4]), 3);
        let check = 6; // "✓" is bytes 6..9
        assert_eq!(complete(&text[..check + 2]), check);
        assert_eq!(complete(&text[..text.len() - 1]), text.len() - 4);
        assert_eq!(complete(&[]), 0);
        // Stray continuation bytes and invalid lead bytes never wait.
        assert_eq!(complete(&[0x61, 0x80, 0x80, 0x80]), 4);
        assert_eq!(complete(&[0x61, 0xFF]), 2);
    }

    #[test]
    fn encode_string_matches_known_vector() {
        let enc = base64_encode(&[Value::String("hello world".into())]);
        let Value::String(s) = &enc else { panic!() };
        assert_eq!(s.to_string(), "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn roundtrip_all_byte_values() {
        let arr = Value::Array(VmRef::new(
            (0u8..=255).map(|b| Value::Number(b as f64)).collect(),
        ));
        let enc = base64_encode(&[arr]);
        assert_eq!(to_bytes(base64_decode(&[enc])), (0u8..=255).collect::<Vec<u8>>());
    }

    #[test]
    fn url_safe_has_no_pad_plus_or_slash() {
        // 0xFB,0xFF,0xBF encodes to +/ in standard; url-safe must use -_ and drop padding.
        let arr = Value::Array(VmRef::new(vec![
            Value::Number(251.0),
            Value::Number(255.0),
            Value::Number(191.0),
        ]));
        let Value::String(sv) = base64_url_encode(&[arr]) else { panic!() };
        let s = sv.to_string();
        assert!(!s.contains('=') && !s.contains('+') && !s.contains('/'), "got {s}");
        // and it round-trips
        assert_eq!(to_bytes(base64_url_decode(&[Value::String(sv)])), vec![251u8, 255, 191]);
    }

    #[test]
    fn decode_invalid_is_null() {
        assert!(matches!(
            base64_decode(&[Value::String("!!! not base64 !!!".into())]),
            Value::Null
        ));
    }

    #[test]
    fn utf8_roundtrip_multibyte() {
        // "héllo 🚀" exercises 2-byte (é) and 4-byte (🚀) sequences.
        let bytes = utf8_encode(&[Value::String("héllo 🚀".into())]);
        let Value::String(s) = utf8_decode(&[bytes]) else { panic!() };
        assert_eq!(s.to_string(), "héllo 🚀");
    }
}
