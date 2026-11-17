//! Bounded stringification of values that user code controls.
//!
//! Every conversion here runs potentially user-supplied Python code
//! (`__str__`, `__repr__`) and every one of them is contained: a raising hook
//! is recorded and captured as a placeholder rather than propagated into the
//! logging call.
//!
//! Containing a *failure* is not enough on its own, because a conversion that
//! succeeds can still build an arbitrarily large string. The built-in string and
//! byte types are therefore recognised before the general path: their exact
//! content is sliced down to the ceiling first, and the slice — not the original
//! value — is what gets escaped or copied. Rendering a one-megabyte `str`
//! argument to `repr` therefore costs the ceiling, not the argument.
//!
//! A sliced `str` is handed back to Python's `repr`, so quote selection and
//! `\xNN` escapes match what a caller would see from an untruncated payload. A
//! `bytes` or `bytearray` is escaped here byte by byte for the same reason: the
//! `repr` of a large blob would be paid for in full before the ceiling could
//! apply, and a lossy decode would not reproduce CPython's escapes.

use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PySlice, PyString, PyStringMethods};

use super::limits::{TRUNCATION_SUFFIX, TRUNCATION_SUFFIX_LEN, truncate_to_limit};

/// Largest byte string rendered inline rather than summarised.
///
/// Escaping expands one byte to as many as four characters, so this ceiling also
/// bounds the work a large `bytes` or `bytearray` argument can demand.
const MAX_INLINE_BLOB: usize = 512;

/// A value that was converted to text, together with how it was obtained.
pub(super) struct BoundedText {
    /// Retained text, already inside the caller's byte ceiling.
    pub text: String,
    /// `true` when the source value was longer than the ceiling allowed.
    pub truncated: bool,
    /// `true` when converting the value raised and [`BoundedText::text`] holds
    /// a placeholder instead of the real value.
    pub failed: bool,
}

impl BoundedText {
    /// Record a conversion failure with `placeholder` inside the byte ceiling.
    fn failure(placeholder: &str, limit: usize) -> Self {
        let truncated = placeholder.len() > limit;
        let text = truncate_to_limit(placeholder, limit).unwrap_or_else(|| placeholder.to_string());
        Self {
            text,
            truncated,
            failed: true,
        }
    }

    /// Record a successful conversion of `text`.
    fn ok(text: String, truncated: bool) -> Self {
        Self {
            text,
            truncated,
            failed: false,
        }
    }
}

/// Apply `limit` to `text`, reporting whether the marker was added.
pub(super) fn clamp(text: String, limit: usize) -> BoundedText {
    match truncate_to_limit(&text, limit) {
        Some(shortened) => BoundedText::ok(shortened, true),
        None => BoundedText::ok(text, false),
    }
}

/// Convert `value` to `str`, containing any failure.
///
/// `placeholder` is retained when the conversion raises. This is the outermost
/// containment point for exception messages.
pub(super) fn stringify(value: &Bound<'_, PyAny>, limit: usize, placeholder: &str) -> BoundedText {
    let Ok(rendered) = value.str() else {
        return BoundedText::failure(placeholder, limit);
    };
    bounded_string(&rendered, limit)
}

/// Convert `value` to `repr`, containing any failure.
///
/// Returns `None` when the value has no usable representation; callers treat
/// that as an element to skip and count.
pub(super) fn repr_bounded(value: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    if let Some(rendered) = render_builtin(value, limit) {
        return Some(rendered);
    }
    let binding = value.repr().ok()?;
    Some(bounded_string(&binding, limit))
}

/// Convert a built-in string to bounded UTF-8 without copying its full value.
///
/// Python strings can contain lone surrogates, which `to_str()` cannot borrow
/// as UTF-8. For those values the bounded prefix is sliced in Python first,
/// then converted lossily; the conversion therefore allocates in proportion to
/// `limit`, even when the original string is very large.
pub(super) fn bounded_string(value: &Bound<'_, PyString>, limit: usize) -> BoundedText {
    let Ok(code_points) = value.code_point_len() else {
        return BoundedText::failure("<string conversion failed>", limit);
    };
    if code_points <= limit {
        return clamp(value.to_string_lossy().into_owned(), limit);
    }

    let stop = isize::try_from(limit).unwrap_or(isize::MAX);
    let slice = PySlice::new(value.py(), 0, stop, 1);
    let Ok(prefix) = value.as_any().get_item(slice) else {
        return BoundedText::failure("<string conversion failed>", limit);
    };
    let Ok(prefix) = prefix.cast::<PyString>() else {
        return BoundedText::failure("<string conversion failed>", limit);
    };
    let text = prefix.to_string_lossy().into_owned();
    BoundedText::ok(force_truncate(&text, limit), true)
}

/// Convert the single argument used by `BaseException.__str__` safely.
///
/// `str(bytes)` and `str(bytearray)` construct representations of the complete
/// value in CPython. Render their bounded prefix directly so a large binary
/// argument cannot allocate an oversized intermediate message.
pub(super) fn stringify_argument(value: &Bound<'_, PyAny>, limit: usize) -> BoundedText {
    if let Ok(text) = value.cast::<PyString>() {
        return bounded_string(text, limit);
    }
    if let Some((rendered, truncated)) = render_byte_like(value) {
        return bounded_rendered(rendered, truncated, limit);
    }
    stringify(value, limit, "<exception str() failed>")
}

/// Render a `bytes` or `bytearray` value from a bounded copy.
///
/// Returns `None` for any other type. An oversized `bytearray` is summarised
/// from its length alone, so its buffer is copied only when it is small enough
/// to render inline and no borrowed slice outlives the check.
fn render_byte_like(value: &Bound<'_, PyAny>) -> Option<(String, bool)> {
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Some(render_blob(bytes.as_bytes()));
    }
    let bytearray = value.cast::<PyByteArray>().ok()?;
    if bytearray.len() > MAX_INLINE_BLOB {
        // Summarise from the length alone, like the inline path's wrapper, so
        // the type name survives without copying the buffer.
        let (summary, _) = oversized_blob();
        return Some((format!("bytearray({summary})"), true));
    }
    let (rendered, truncated) = render_blob(&bytearray.to_vec());
    Some((format!("bytearray({rendered})"), truncated))
}

/// The placeholder used for a byte string too large to render inline.
fn oversized_blob() -> (String, bool) {
    ("b'<binary blob>'".to_string(), true)
}

/// Clamp an already-rendered built-in and preserve whether its source was cut.
fn bounded_rendered(rendered: String, was_truncated: bool, limit: usize) -> BoundedText {
    if was_truncated {
        return BoundedText::ok(truncate_literal(&rendered, limit), true);
    }
    clamp(rendered, limit)
}

/// Cut a prefix and include the ASCII marker whenever the ceiling permits it.
fn force_truncate(value: &str, limit: usize) -> String {
    let suffix_bytes = if limit >= TRUNCATION_SUFFIX_LEN {
        TRUNCATION_SUFFIX_LEN
    } else {
        0
    };
    let prefix = prefix_within(value, limit.saturating_sub(suffix_bytes));
    let mut truncated = String::with_capacity(prefix.len() + suffix_bytes);
    truncated.push_str(prefix);
    truncated.push_str(&TRUNCATION_SUFFIX[..suffix_bytes]);
    truncated
}

/// Render a built-in string or byte-like value from a bounded slice.
///
/// Returns `None` for any other type, leaving the caller to take the general
/// `repr` path.
fn render_builtin(value: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    if let Some((rendered, truncated)) = render_byte_like(value) {
        return Some(bounded_rendered(rendered, truncated, limit));
    }
    render_long_string(value, limit)
}

/// Render an oversized `str` from a bounded prefix.
///
/// Returns `None` when the value is not a `str`, still fits the ceiling, or is
/// not exposed as UTF-8 — a string holding lone surrogates — leaving the caller
/// to take the general path.
///
/// The prefix leaves room for the two quotes and the truncation marker, so the
/// assembled literal lands inside the ceiling rather than being cut a second
/// time; escapes that expand the prefix past that allowance are still caught by
/// [`truncate_literal`], which keeps the literal's closing quote.
fn render_long_string(value: &Bound<'_, PyAny>, limit: usize) -> Option<BoundedText> {
    let text = value.cast::<PyString>().ok()?;
    let Ok(code_points) = text.code_point_len() else {
        return Some(BoundedText::failure("<repr conversion failed>", limit));
    };
    if code_points <= limit {
        return None;
    }
    let reserve = TRUNCATION_SUFFIX_LEN.saturating_add(2);
    let stop = isize::try_from(limit.saturating_sub(reserve)).unwrap_or(isize::MAX);
    let slice = PySlice::new(value.py(), 0, stop, 1);
    let Ok(prefix) = value.get_item(slice) else {
        return Some(BoundedText::failure("<repr conversion failed>", limit));
    };
    let Ok(prefix) = prefix.cast::<PyString>() else {
        return Some(BoundedText::failure("<repr conversion failed>", limit));
    };
    let Ok(rendered) = prefix.repr() else {
        return Some(BoundedText::failure("<repr conversion failed>", limit));
    };
    let rendered = rendered.to_string_lossy();
    Some(BoundedText::ok(truncate_literal(&rendered, limit), true))
}

/// Cut `value` to at most `budget` bytes on a character boundary.
fn prefix_within(value: &str, budget: usize) -> &str {
    let mut cut = budget.min(value.len());
    while cut > 0 && !value.is_char_boundary(cut) {
        cut -= 1;
    }
    &value[..cut]
}

/// Shorten a rendered literal to `limit` bytes, keeping its closing quote.
///
/// The marker goes inside the literal's quotes, and any wrapper suffix such as
/// the `)` of `bytearray(...)` is kept after them. Cutting the assembled text
/// from the end instead would strip the closing quote, leaving something that
/// no longer reads as Python; the quote and wrapper survive even when the
/// ceiling leaves no room for the marker.
fn truncate_literal(rendered: &str, limit: usize) -> String {
    let Some(quote_start) = rendered.rfind(['\'', '"']) else {
        return force_truncate(rendered, limit);
    };
    let quote = &rendered[quote_start..quote_start + 1];
    let body = &rendered[..quote_start];
    let tail = &rendered[quote_start + 1..];
    let closing = quote.len() + tail.len();
    let overhead = TRUNCATION_SUFFIX_LEN + closing;
    if limit <= overhead {
        let budget = limit.saturating_sub(closing);
        return format!("{}{quote}{tail}", prefix_within(body, budget));
    }
    let budget = limit - overhead;
    format!(
        "{}{TRUNCATION_SUFFIX}{quote}{tail}",
        prefix_within(body, budget)
    )
}

/// Render a byte string in CPython's own `bytes` repr form.
///
/// CPython escapes a byte string itself rather than decoding it, so the bytes
/// are escaped here one at a time instead of being lossily decoded first. A
/// lossy decode would fold every high byte into U+FFFD and lose the value; it
/// also cannot express `\x7f` or `\x00` in the form CPython prints them. The
/// rules reproduced are CPython's: printable ASCII stands for itself, `\\` and
/// `'` are escaped, `\t`, `\n` and `\r` use their short forms, and everything
/// else becomes `\xNN`. The quote flips to `"` when the contents hold a `'` but
/// no `"`, so the result stays a valid Python literal either way.
fn render_blob(raw: &[u8]) -> (String, bool) {
    if raw.len() > MAX_INLINE_BLOB {
        return oversized_blob();
    }
    let quote = if raw.contains(&b'\'') && !raw.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(raw.len() + 3);
    rendered.push('b');
    rendered.push(quote);
    for byte in raw {
        push_escaped_byte(&mut rendered, *byte, quote);
    }
    rendered.push(quote);
    (rendered, false)
}

/// Append one byte of a byte string in CPython's repr form.
fn push_escaped_byte(rendered: &mut String, byte: u8, quote: char) {
    match byte {
        b'\\' => rendered.push_str("\\\\"),
        b'\t' => rendered.push_str("\\t"),
        b'\n' => rendered.push_str("\\n"),
        b'\r' => rendered.push_str("\\r"),
        // The closing quote is only ever the active one, so the other quote
        // character is left to stand for itself.
        0x20..=0x7e => match char::from(byte) {
            ch if ch == quote => {
                rendered.push('\\');
                rendered.push(ch);
            }
            ch => rendered.push(ch),
        },
        other => {
            // `\xNN` is how CPython prints every other byte, including DEL and
            // every byte above the ASCII range.
            rendered.push_str("\\x");
            rendered.push(char::from_digit(u32::from(other) >> 4, 16).unwrap_or('0'));
            rendered.push(char::from_digit(u32::from(other) & 0xf, 16).unwrap_or('0'));
        }
    }
}
