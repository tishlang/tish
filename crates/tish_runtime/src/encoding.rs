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
}
