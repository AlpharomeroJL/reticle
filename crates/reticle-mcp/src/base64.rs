//! Minimal standard base64 encoding (RFC 4648), encode-only.
//!
//! The only binary payload the server hands back as text is a rendered PNG (see
//! [`crate::context`]), so a full base64 dependency would be overkill. This is a
//! small, allocation-once encoder over the standard alphabet with `=` padding,
//! matching what an MCP client expects for an inline image.

/// The standard base64 alphabet (RFC 4648, table 1).
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `bytes` as a standard base64 string with `=` padding.
///
/// Each three input bytes become four output characters; a trailing group of one
/// or two bytes is padded to four characters with `=`.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    // Four output chars per three input bytes, rounded up.
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n >> 6 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
    }
    match chunks.remainder() {
        [a] => {
            let n = u32::from(*a) << 16;
            out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
            out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        [a, b] => {
            let n = (u32::from(*a) << 16) | (u32::from(*b) << 8);
            out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
            out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
            out.push(ALPHABET[(n >> 6 & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode;

    /// The RFC 4648 section 10 test vectors pin the alphabet and the padding of
    /// each remainder length.
    #[test]
    fn rfc4648_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    /// A byte that exercises the high bits of the alphabet (`/`, `+`).
    #[test]
    fn high_bits() {
        assert_eq!(encode(&[0xff, 0xff, 0xff]), "////");
        assert_eq!(encode(&[0xfb, 0xff, 0xbf]), "+/+/");
    }
}
