//! URI encoding/decoding utilities.

/// Percent-decode a string (for decodeURI).
/// Does NOT decode reserved URI characters: ; / ? : @ & = + $ , #
/// These are characters that encodeURI does not encode, so decodeURI won't decode them.
pub fn percent_decode(input: &str) -> Result<String, String> {
    // Reserved characters that decodeURI should NOT decode (because encodeURI doesn't encode them)
    const RESERVED_ENCODED: &[&str] = &[
        "%3B", "%3b", // ;
        "%2F", "%2f", // /
        "%3F", "%3f", // ?
        "%3A", "%3a", // :
        "%40", // @
        "%26", // &
        "%3D", "%3d", // =
        "%2B", "%2b", // +
        "%24", // $
        "%2C", "%2c", // ,
        "%23", // #
    ];

    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            // Peek at the next two characters to check if this is a reserved sequence
            let mut hex = String::new();
            let mut peek_chars = Vec::new();
            for _ in 0..2 {
                match chars.next() {
                    Some(h) if h.is_ascii_hexdigit() => {
                        hex.push(h);
                        peek_chars.push(h);
                    }
                    Some(h) => {
                        // Not a valid hex sequence, push as-is
                        result.push('%');
                        for pc in peek_chars {
                            result.push(pc);
                        }
                        result.push(h);
                        hex.clear();
                        break;
                    }
                    None => return Err("URIError: malformed URI sequence".to_string()),
                }
            }

            if hex.len() == 2 {
                let encoded = format!("%{}", hex);
                // Check if this is a reserved character that should NOT be decoded
                if RESERVED_ENCODED
                    .iter()
                    .any(|r| r.eq_ignore_ascii_case(&encoded))
                {
                    result.push_str(&encoded);
                } else if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                }
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

    let mut result = String::with_capacity(input.len());
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

    fn percent_encode_component(input: &str) -> String {
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

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = "hello world";
        let encoded = percent_encode_component(original);
        assert_eq!(encoded, "hello%20world");
        let decoded = percent_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
