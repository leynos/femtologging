//! Escaping of message line separators for the default text formatter.
//!
//! The default format puts one record on one physical line, so message text
//! must not contain characters that a reader or Python's `str.splitlines()`
//! treats as a line boundary. Escaping those characters keeps a message from
//! impersonating generated event metadata on a following line.

use std::borrow::Cow;

/// Characters that Python's `str.splitlines()` treats as line boundaries.
///
/// Mirroring CPython keeps escaped output free of any character that would
/// start a new line when Python later inspects it. Compound endings such as
/// CRLF are escaped one character at a time.
const LINE_SEPARATORS: [char; 10] = [
    '\u{000A}', // LINE FEED
    '\u{000B}', // LINE TABULATION (VERTICAL TAB)
    '\u{000C}', // FORM FEED
    '\u{000D}', // CARRIAGE RETURN
    '\u{001C}', // FILE SEPARATOR
    '\u{001D}', // GROUP SEPARATOR
    '\u{001E}', // RECORD SEPARATOR
    '\u{0085}', // NEXT LINE (NEL)
    '\u{2028}', // LINE SEPARATOR
    '\u{2029}', // PARAGRAPH SEPARATOR
];

/// Return whether `character` is one of the escaped line separators.
fn is_line_separator(character: char) -> bool {
    LINE_SEPARATORS.contains(&character)
}

/// Escape every line separator in `message` using its `char::escape_debug` form.
///
/// All other characters, including backslashes and horizontal tabs, are kept
/// verbatim, so an already escaped sequence such as a literal `\n` stays
/// unchanged. The result is therefore not reversible, but it is unambiguous as
/// a single physical line. The input is borrowed when it contains no separator,
/// so the common single-line case does not allocate.
///
/// ```text
/// escape_message_line_separators("a\nb") == "a\\nb"
/// escape_message_line_separators("a\\nb") == "a\\nb"
/// ```
pub(crate) fn escape_message_line_separators(message: &str) -> Cow<'_, str> {
    if !message.chars().any(is_line_separator) {
        return Cow::Borrowed(message);
    }

    let mut escaped = String::with_capacity(message.len());
    for character in message.chars() {
        if is_line_separator(character) {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    //! Tests for message line-separator escaping.

    use std::borrow::Cow;

    use rstest::rstest;

    use super::escape_message_line_separators;

    #[test]
    fn borrows_message_without_separators() {
        let escaped = escape_message_line_separators("plain message");

        assert!(matches!(escaped, Cow::Borrowed("plain message")));
    }

    #[rstest]
    #[case("\n", "\\n", "line feed")]
    #[case("\r", "\\r", "carriage return")]
    #[case("\u{000B}", "\\u{b}", "line tabulation")]
    #[case("\u{000C}", "\\u{c}", "form feed")]
    #[case("\u{001C}", "\\u{1c}", "file separator")]
    #[case("\u{001D}", "\\u{1d}", "group separator")]
    #[case("\u{001E}", "\\u{1e}", "record separator")]
    #[case("\u{0085}", "\\u{85}", "next line")]
    #[case("\u{2028}", "\\u{2028}", "line separator")]
    #[case("\u{2029}", "\\u{2029}", "paragraph separator")]
    fn escapes_each_line_separator(
        #[case] separator: &str,
        #[case] expected: &str,
        #[case] description: &str,
    ) {
        let message = format!("before{separator}after");

        let escaped = escape_message_line_separators(&message);

        assert_eq!(escaped, format!("before{expected}after"), "{description}");
    }

    #[test]
    fn escapes_crlf_as_two_separate_sequences() {
        let escaped = escape_message_line_separators("first\r\nsecond");

        assert_eq!(escaped, "first\\r\\nsecond");
    }

    #[test]
    fn escapes_trailing_separators() {
        let escaped = escape_message_line_separators("trailing\n");

        assert_eq!(escaped, "trailing\\n");
    }

    #[test]
    fn keeps_literal_backslash_sequences_unchanged() {
        let escaped = escape_message_line_separators(r"literal\nsequence");

        assert_eq!(escaped, r"literal\nsequence");
    }

    #[test]
    fn keeps_horizontal_tab_unchanged() {
        let escaped = escape_message_line_separators("column\tvalue");

        assert_eq!(escaped, "column\tvalue");
    }

    #[test]
    fn leaves_empty_message_empty() {
        let escaped = escape_message_line_separators("");

        assert_eq!(escaped, "");
    }
}
