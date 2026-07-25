//! Cryptographic hashing + secure random for Tish (`tish:crypto`), behind the `crypto` feature.
//!
//!   import { sha256, sha256Hex, randomBytes } from 'tish:crypto'
//!
//!   - `sha256(bytesOrString) -> number[]`   (the 32-byte digest as a byte array)
//!   - `sha256Hex(bytesOrString) -> string`  (64 lowercase hex chars)
//!   - `randomBytes(n) -> number[]`          (n cryptographically-secure random bytes; n clamped to 1 MiB)
//!
//! Bytes are the `tish:fs` / `tish:encoding` representation (Numbers 0-255). A string argument is
//! hashed as its UTF-8 bytes. For PKCE: `base64UrlEncode(sha256(verifier))` (see `tish:encoding`).

use sha2::{Digest, Sha256};
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

fn bytes_to_value(bytes: &[u8]) -> Value {
    Value::Array(VmRef::new(
        bytes.iter().map(|b| Value::Number(*b as f64)).collect(),
    ))
}

fn digest_of(args: &[Value]) -> Option<[u8; 32]> {
    let b = arg_bytes(args)?;
    let mut h = Sha256::new();
    h.update(&b);
    Some(h.finalize().into())
}

pub fn sha256(args: &[Value]) -> Value {
    match digest_of(args) {
        Some(d) => bytes_to_value(&d),
        None => Value::Null,
    }
}

pub fn sha256_hex(args: &[Value]) -> Value {
    match digest_of(args) {
        Some(d) => {
            let mut s = String::with_capacity(64);
            for byte in d.iter() {
                s.push_str(&format!("{byte:02x}"));
            }
            Value::String(s.into())
        }
        None => Value::Null,
    }
}

pub fn random_bytes(args: &[Value]) -> Value {
    let n = match args.first() {
        Some(Value::Number(x)) if x.is_finite() && *x >= 0.0 => (*x as usize).min(1_048_576),
        _ => return Value::Null,
    };
    // rand::random draws from the thread CSPRNG (ChaCha, OS-seeded) — same source fs_ext uses.
    let bytes: Vec<u8> = (0..n).map(|_| rand::random::<u8>()).collect();
    bytes_to_value(&bytes)
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
    fn sha256_hex_known_vector() {
        // The canonical NIST vector: SHA-256("abc").
        let Value::String(s) = sha256_hex(&[Value::String("abc".into())]) else { panic!() };
        assert_eq!(
            s.to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_returns_32_bytes_matching_hex() {
        let bytes = to_bytes(sha256(&[Value::String("abc".into())]));
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes[0], 0xba);
        assert_eq!(bytes[31], 0xad);
    }

    #[test]
    fn sha256_accepts_byte_array() {
        // "abc" as a byte array must hash identically to the string.
        let arr = Value::Array(VmRef::new(vec![
            Value::Number(97.0),
            Value::Number(98.0),
            Value::Number(99.0),
        ]));
        let Value::String(s) = sha256_hex(&[arr]) else { panic!() };
        assert_eq!(
            s.to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn random_bytes_length_and_variance() {
        assert_eq!(to_bytes(random_bytes(&[Value::Number(0.0)])).len(), 0);
        let a = to_bytes(random_bytes(&[Value::Number(32.0)]));
        let b = to_bytes(random_bytes(&[Value::Number(32.0)]));
        assert_eq!(a.len(), 32);
        assert_ne!(a, b, "two 32-byte draws must (almost surely) differ");
    }
}
