//! URL encoding utilities for HTTP serialization.
//!
//! Provides URL encoding compatible with CPython's `urllib.parse.urlencode`,
//! using `+` for spaces as CPython's `quote_plus` does by default.

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

/// Characters to percent-encode in URL query values (excluding space).
///
/// This encodes all control characters plus characters with special meaning in
/// URLs (query separators, reserved characters), while leaving unreserved
/// characters (alphanumeric, `-`, `_`, `.`, `~`) as-is per RFC 3986.
///
/// Space is handled separately by [`url_encode`] which maps it directly to `+`
/// during iteration, avoiding a second pass over the encoded string.
pub(super) const QUERY_ENCODE_SET_NO_SPACE: &AsciiSet = &CONTROLS
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(b'\'');

/// URL-encode a string using `+` for spaces (CPython `urlencode` parity).
///
/// This matches the behaviour of `urllib.parse.urlencode`, which uses
/// `quote_plus` internally and encodes spaces as `+` rather than `%20`.
///
/// Spaces are mapped to `+` directly during encoding (single pass), rather than
/// encoding to `%20` and then replacing in a second pass.
pub(super) fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut first = true;
    for chunk in s.split(' ') {
        if !first {
            result.push('+');
        }
        first = false;
        result.push_str(&utf8_percent_encode(chunk, QUERY_ENCODE_SET_NO_SPACE).to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_special_chars() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("test_value-123.txt"), "test_value-123.txt");
    }

    #[test]
    fn url_encode_edge_cases() {
        // Empty string
        assert_eq!(url_encode(""), "");
        // Consecutive spaces
        assert_eq!(url_encode("a  b"), "a++b");
        assert_eq!(url_encode("a   b"), "a+++b");
        // Leading spaces
        assert_eq!(url_encode(" hello"), "+hello");
        assert_eq!(url_encode("  hello"), "++hello");
        // Trailing spaces
        assert_eq!(url_encode("hello "), "hello+");
        assert_eq!(url_encode("hello  "), "hello++");
        // Only spaces
        assert_eq!(url_encode(" "), "+");
        assert_eq!(url_encode("  "), "++");
        assert_eq!(url_encode("   "), "+++");
    }
}
