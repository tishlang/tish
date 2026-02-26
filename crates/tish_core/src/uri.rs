//! URI encoding/decoding utilities.

/// Percent-decode a string (for decodeURI/decodeURIComponent).
pub fn percent_decode(input: &str) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            let mut hex = String::new();
            for _ in 0..2 {
                match chars.next() {
                    Some(h) if h.is_ascii_hexdigit() => hex.push(h),
                    _ => return Err("URIError: malformed URI sequence".to_string()),
                }
            }
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Percent-encode a string (for encodeURI).
/// Preserves: A-Z a-z 0-9 - _ . ! ~ * ' ( ) ; / ? : @ & = + $ , #
pub fn percent_encode(input: &str) -> String {
    const UNRESERVED: &[char] = &[
        '-', '_', '.', '!', '~', '*', '\'', '(', ')', ';', '/', '?', ':', '@', '&', '=', '+', '$',
        ',', '#',
    ];

    let mut result = String::new();
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || UNRESERVED.contains(&c) {
            result.push(c);
        } else {
            for byte in c.to_string().as_bytes() {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// Percent-encode for encodeURIComponent.
/// Preserves: A-Z a-z 0-9 - _ . ! ~ * ' ( )
pub fn percent_encode_component(input: &str) -> String {
    const UNRESERVED: &[char] = &['-', '_', '.', '!', '~', '*', '\'', '(', ')'];

    let mut result = String::new();
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || UNRESERVED.contains(&c) {
            result.push(c);
        } else {
            for byte in c.to_string().as_bytes() {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = "hello world";
        let encoded = percent_encode_component(original);
        assert_eq!(encoded, "hello%20world");
        let decoded = percent_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
