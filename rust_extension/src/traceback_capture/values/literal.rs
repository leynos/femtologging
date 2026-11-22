//! Literal-aware rendering and clipping for built-in string and byte values.
//!
//! `values` decides *what* to render and this module decides how a rendered
//! literal is escaped and shortened. Keeping the escaping rules here leaves the
//! parent module under the repository's module-size ceiling without splitting
//! the two concerns across unrelated files.
//!
//! A literal is clipped on its own boundaries: the opening quote, the closing
//! quote and any wrapper such as the `)` of `bytearray(...)` are reserved, and
//! the body is cut only between complete escapes. A plain byte cut could leave
//! half an escape and no closing quote, which no longer reads as Python.

use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PySlice};

use super::BoundedText;
use crate::traceback_capture::limits::{TRUNCATION_SUFFIX, TRUNCATION_SUFFIX_LEN};

/// Largest byte string rendered inline rather than summarised.
///
/// Escaping expands one byte to as many as four characters, so this ceiling also
/// bounds the work a large `bytes` or `bytearray` argument can demand.
const MAX_INLINE_BLOB: usize = 512;

/// Render a `bytes` or `bytearray` value from a bounded copy.
///
/// Returns `None` for any other type, including subclasses of `bytes` and
/// `bytearray`: a subclass may override `__str__` or `__repr__`, so rendering
/// its buffer directly would ignore the representation it defines. An oversized
/// exact `bytearray` is summarised from its length alone, so its buffer is
/// copied only when it is small enough to render inline and no borrowed slice
/// outlives the check.
pub(super) fn render_byte_like(value: &Bound<'_, PyAny>) -> Option<(String, bool)> {
    if value.is_exact_instance_of::<PyBytes>()
        && let Ok(bytes) = value.cast::<PyBytes>()
    {
        return Some(render_blob(bytes.as_bytes()));
    }
    if !value.is_exact_instance_of::<PyByteArray>() {
        return None;
    }
    // The buffer may be resized by another thread between a length check and a
    // copy, so the snapshot is taken first and its own length decides the path.
    // A slice of at most `MAX_INLINE_BLOB + 1` bytes costs the ceiling whatever
    // the original size was, and one extra byte is enough to tell "fits" from
    // "oversized".
    let snapshot = bounded_bytearray_snapshot(value)?;
    if snapshot.len() > MAX_INLINE_BLOB {
        // Summarise from the length alone, like the inline path's wrapper, so
        // the type name survives without copying the buffer.
        let (summary, _) = oversized_blob();
        return Some((format!("bytearray({summary})"), true));
    }
    // `repr` of the bounded snapshot follows CPython's `bytearray` rules,
    // including its choice to keep a double-quoted literal escaped, which the
    // shared `bytes` renderer does not reproduce.
    let rendered = snapshot.repr().ok()?.to_string_lossy().into_owned();
    Some((rendered, false))
}

/// Copy at most `MAX_INLINE_BLOB + 1` bytes of a bytearray.
///
/// Slicing through the unbound `bytearray.__getitem__` performs one bounded
/// copy inside the interpreter, so a concurrently resizing buffer cannot make
/// the copied length differ from the checked one. Slicing the instance would
/// dispatch to a subclass's `__getitem__`, which is why the type's own method
/// is called.
fn bounded_bytearray_snapshot<'py>(value: &Bound<'py, PyAny>) -> Option<Bound<'py, PyByteArray>> {
    let py = value.py();
    let getitem = py.get_type::<PyByteArray>().getattr("__getitem__").ok()?;
    let stop = isize::try_from(MAX_INLINE_BLOB.saturating_add(1)).unwrap_or(isize::MAX);
    let slice = PySlice::new(py, 0, stop, 1);
    getitem.call1((value, slice)).ok()?.cast_into().ok()
}

/// The placeholder used for a byte string too large to render inline.
fn oversized_blob() -> (String, bool) {
    ("b'<binary blob>'".to_string(), true)
}

/// Clamp an already-rendered built-in and preserve whether its source was cut.
///
/// A rendered literal that exceeds the ceiling is cut on a literal boundary
/// even when its source was not sliced: a short string of NULs can fit the
/// input ceiling while its escaped form does not, and a plain cut would split
/// an escape and drop the closing quote.
pub(super) fn bounded_rendered(rendered: String, was_truncated: bool, limit: usize) -> BoundedText {
    if was_truncated || rendered.len() > limit {
        return BoundedText::ok(truncate_literal(&rendered, limit), true);
    }
    BoundedText::ok(rendered, false)
}

/// Cut a prefix and include the ASCII marker whenever the ceiling permits it.
pub(super) fn force_truncate(value: &str, limit: usize) -> String {
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
///
/// The body is cut with [`truncated_body`], so the shortened literal never ends
/// halfway through an escape such as `\x41`. When the ceiling cannot even hold
/// the closing quote and wrapper, the ceiling wins and the text is cut from the
/// end: a truncated literal is better than one that claims a close it lost.
fn truncate_literal(rendered: &str, limit: usize) -> String {
    let Some(open) = rendered.find(['\'', '"']) else {
        return force_truncate(rendered, limit);
    };
    let Some(close) = rendered.rfind(['\'', '"']) else {
        return force_truncate(rendered, limit);
    };
    if close <= open {
        return force_truncate(rendered, limit);
    }
    // The opening quote and any wrapper before it (`b`, `bytearray(b`) are the
    // literal's identity; cutting inside them yields text that no longer reads
    // as a literal at all. They are reserved together with the closing quote
    // and wrapper before the body is sized.
    let head = &rendered[..=open];
    let body = &rendered[open + 1..close];
    let quote = &rendered[close..close + 1];
    let tail = &rendered[close + 1..];
    let fixed = head.len() + quote.len() + tail.len();
    if fixed > limit {
        return force_truncate(rendered, limit);
    }
    if limit < fixed + TRUNCATION_SUFFIX_LEN {
        let budget = limit - fixed;
        return format!("{head}{}{quote}{tail}", truncated_body(body, budget));
    }
    let budget = limit - fixed - TRUNCATION_SUFFIX_LEN;
    format!(
        "{head}{}{TRUNCATION_SUFFIX}{quote}{tail}",
        truncated_body(body, budget)
    )
}

/// Longest prefix of a literal body within `budget` that ends between escapes.
///
/// An escape that would extend past the budget is dropped whole, because a cut
/// escape such as `\x4` no longer means the character it started to describe.
fn truncated_body(body: &str, budget: usize) -> &str {
    let mut index = 0;
    let mut safe = 0;
    while index < body.len() && index < budget {
        let Some(ch) = body[index..].chars().next() else {
            break;
        };
        let width = if ch == '\\' {
            let Some(width) = completed_escape(&body[index..]) else {
                break;
            };
            width
        } else {
            ch.len_utf8()
        };
        if index + width > budget {
            break;
        }
        index += width;
        safe = index;
    }
    &body[..safe]
}

/// Byte width of the complete escape sequence at the start of `rest`.
///
/// Returns `None` when `rest` begins with a backslash that is not followed by
/// the rest of its escape, which is how a literal body cut short would look.
fn completed_escape(rest: &str) -> Option<usize> {
    let width = match rest.as_bytes().get(1)? {
        b'x' => 4,
        b'u' => 6,
        b'U' => 10,
        _ => 2,
    };
    (rest.len() >= width).then_some(width)
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

#[cfg(test)]
mod tests {
    //! Literal clipping at its boundaries.

    use super::*;

    #[test]
    fn an_exact_fit_still_shows_the_marker() {
        // Two quotes and the marker fill five bytes exactly; dropping the
        // marker here would make a truncated value look complete.
        assert_eq!(truncate_literal("'abcd'", 5), "'...'");
    }

    #[test]
    fn a_wrapper_and_opening_quote_survive_clipping() {
        let clipped = truncate_literal("bytearray(b'<binary blob>')", 16);

        assert!(clipped.len() <= 16, "clipping must respect the ceiling");
        assert!(
            clipped.starts_with("bytearray(b'"),
            "the wrapper and opening quote must survive: {clipped:?}"
        );
        assert!(
            clipped.ends_with("')"),
            "the closing quote and wrapper must survive: {clipped:?}"
        );
    }
}
