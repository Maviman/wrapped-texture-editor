// Owner: core-import (C5). URI helpers of ARCHITECTURE.md 5.21: base64, percent-decoding and `data:` URIs
// (glTF buffers and images embedded in the JSON). Hand-written: the workspace has no base64 / url crate.

/// Value of one base64 character of the standard alphabet.
fn base64_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Base64 decode: standard alphabet, padding optional, whitespace ignored.
///
/// `None` for any other character, for data after the padding, for more than two `=`, for padding that does not
/// complete a group of four and for a dangling single character. Unused low bits of the last group are ignored.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    let (mut chars, mut padding) = (0usize, 0usize);
    for &c in text.as_bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding += 1;
            if padding > 2 {
                return None;
            }
            continue;
        }
        if padding > 0 {
            return None;
        }
        acc = (acc << 6) | base64_value(c)?;
        bits += 6;
        chars += 1;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if chars % 4 == 1 || (padding > 0 && (chars + padding) % 4 != 0) {
        return None;
    }
    Some(out)
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Percent-decoding to raw bytes; invalid or truncated escapes are kept literally.
pub(crate) fn percent_decode_bytes(text: &str) -> Vec<u8> {
    let src = text.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'%'
            && i + 2 < src.len()
            && let (Some(hi), Some(lo)) = (hex_value(src[i + 1]), hex_value(src[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(src[i]);
        i += 1;
    }
    out
}

/// Percent-decode: invalid escapes are kept literally; lossy UTF-8.
pub fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    String::from_utf8_lossy(&percent_decode_bytes(text)).into_owned()
}

/// Parses a `data:` URI into (mime type, decoded bytes).
///
/// Grammar: `data:[<mime type>][;<parameter>]*[;base64],<payload>`; the scheme is matched case-insensitively. The
/// mime type is `None` when it is omitted. A base64 payload may itself be percent-encoded; any other payload is
/// percent-decoded. `None` when the string is not a data URI, has no comma or its base64 payload is malformed.
pub fn parse_data_uri(uri: &str) -> Option<(Option<&str>, Vec<u8>)> {
    let uri = uri.trim_start();
    let scheme = uri.get(..5)?;
    if !scheme.eq_ignore_ascii_case("data:") {
        return None;
    }
    let (header, payload) = uri[5..].split_once(',')?;
    let is_base64 = header.split(';').any(|p| p.trim().eq_ignore_ascii_case("base64"));
    let mime = header
        .split(';')
        .next()
        .map(str::trim)
        .filter(|m| !m.is_empty() && !m.contains('=') && !m.eq_ignore_ascii_case("base64"));
    let bytes = if is_base64 {
        match base64_decode(payload) {
            Some(bytes) => bytes,
            // "%2B", "%2F" and "%3D" are legal spellings of '+', '/' and '=' inside a URI.
            None if payload.contains('%') => base64_decode(&percent_decode(payload))?,
            None => return None,
        }
    } else {
        percent_decode_bytes(payload)
    };
    Some((mime, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference encoder for the round-trip test.
    fn base64_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("+/+/").unwrap(), [0xfb, 0xff, 0xbf]);
    }

    #[test]
    fn base64_padding_is_optional_and_whitespace_is_ignored() {
        assert_eq!(base64_decode("Zg").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9vYg").unwrap(), b"foob");
        assert_eq!(base64_decode(" Zm9v\r\nYmFy\t\n").unwrap(), b"foobar");
        assert_eq!(base64_decode("Z m 8 =").unwrap(), b"fo");
    }

    #[test]
    fn base64_rejects_malformed_input() {
        assert!(base64_decode("Z").is_none(), "dangling character");
        assert!(base64_decode("Zm9vY").is_none(), "dangling character after a full group");
        assert!(base64_decode("Zm9v!").is_none(), "foreign character");
        assert!(base64_decode("Zm-_").is_none(), "URL-safe alphabet is not the standard one");
        assert!(base64_decode("Zg=a").is_none(), "data after padding");
        assert!(base64_decode("Zg===").is_none(), "too much padding");
        assert!(base64_decode("Zm9v=").is_none(), "padding that completes nothing");
        assert!(base64_decode("Zg=").is_none(), "incomplete padding");
        assert!(base64_decode("=").is_none());
    }

    #[test]
    fn base64_round_trips_every_length() {
        let data: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
        for len in 0..data.len() {
            let encoded = base64_encode(&data[..len]);
            assert_eq!(base64_decode(&encoded).unwrap(), &data[..len], "length {len}");
            assert_eq!(base64_decode(encoded.trim_end_matches('=')).unwrap(), &data[..len], "unpadded length {len}");
        }
    }

    #[test]
    fn percent_decode_cases() {
        assert_eq!(percent_decode("plain.png"), "plain.png");
        assert_eq!(percent_decode("my%20tex.png"), "my tex.png");
        assert_eq!(percent_decode("a%2Fb%5cc"), "a/b\\c");
        assert_eq!(percent_decode("%41%62"), "Ab");
        // UTF-8 sequences are assembled from their bytes.
        assert_eq!(percent_decode("caf%C3%A9"), "caf\u{e9}");
        // Invalid and truncated escapes stay as they are.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("50%2"), "50%2");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(percent_decode("%%41"), "%A");
        // Invalid UTF-8 is replaced, not rejected.
        assert_eq!(percent_decode("%FF"), "\u{fffd}");
        // '+' is not a space in a URI path.
        assert_eq!(percent_decode("a+b"), "a+b");
        assert_eq!(percent_decode(""), "");
    }

    #[test]
    fn data_uri_base64() {
        let (mime, bytes) = parse_data_uri("data:application/octet-stream;base64,AAECAw==").unwrap();
        assert_eq!(mime, Some("application/octet-stream"));
        assert_eq!(bytes, [0, 1, 2, 3]);
        let (mime, bytes) = parse_data_uri("DATA:image/png;charset=x;BASE64,Zm9v").unwrap();
        assert_eq!(mime, Some("image/png"));
        assert_eq!(bytes, b"foo");
        // No mime type at all.
        let (mime, bytes) = parse_data_uri("data:;base64,Zm8=").unwrap();
        assert_eq!(mime, None);
        assert_eq!(bytes, b"fo");
        let (mime, bytes) = parse_data_uri("data:base64,Zm8").unwrap();
        assert_eq!(mime, None);
        assert_eq!(bytes, b"fo");
        // Percent-encoded base64 characters.
        let (_, bytes) = parse_data_uri("data:;base64,%2B%2F%2B%2F").unwrap();
        assert_eq!(bytes, [0xfb, 0xff, 0xbf]);
        // Empty payload.
        assert_eq!(parse_data_uri("data:image/png;base64,").unwrap().1, b"");
    }

    #[test]
    fn data_uri_plain_and_malformed() {
        let (mime, bytes) = parse_data_uri("data:text/plain,hello%20world%00").unwrap();
        assert_eq!(mime, Some("text/plain"));
        assert_eq!(bytes, b"hello world\0");
        let (mime, bytes) = parse_data_uri("data:,a,b").unwrap();
        assert_eq!(mime, None);
        assert_eq!(bytes, b"a,b");
        let (mime, _) = parse_data_uri("data:charset=utf-8,x").unwrap();
        assert_eq!(mime, None, "a parameter is not a mime type");
        assert!(parse_data_uri("data:image/png;base64").is_none(), "no comma");
        assert!(parse_data_uri("data:image/png;base64,@@@").is_none(), "bad base64");
        assert!(parse_data_uri("textures/a.png").is_none());
        assert!(parse_data_uri("dat").is_none());
        assert!(parse_data_uri("").is_none());
        assert!(parse_data_uri("d\u{e9}ta:,x").is_none(), "must not split inside a character");
    }
}
