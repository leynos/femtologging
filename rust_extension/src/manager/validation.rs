//! Validation of logger names before they reach the registry.
//!
//! A logger name is frequently derived from caller-supplied data — a tenant,
//! a user, a plug-in, a route. Every name that passes through here becomes a
//! registry key that lives for the life of the process, so the checks are a
//! resource bound as much as a correctness check.
//!
//! Three properties are enforced:
//!
//! - a byte budget, checked first so a hostile megabyte-long name is refused
//!   without being scanned;
//! - a segment budget, because a name describes a component path and component
//!   paths are shallow, whereas request-derived strings are not;
//! - an ASCII-control-character ban, because names reach line-oriented output
//!   and an embedded newline would let a caller forge a record boundary.
//!
//! The shape checks (empty, leading dot, trailing dot, empty segment) exist so
//! that a name always has a well-defined parent chain.

use pyo3::prelude::*;

/// Maximum length, in UTF-8 bytes, accepted for a logger name.
///
/// The bound keeps every name small enough to quote in an error message and
/// stops a caller turning a request identifier into an unbounded registry key.
pub const MAX_LOGGER_NAME_BYTES: usize = 255;

/// Maximum number of dot-separated segments accepted in a logger name.
///
/// Logger names describe a component hierarchy, which is shallow by nature; a
/// name deeper than this is a request-derived string rather than a component
/// path.
pub const MAX_LOGGER_NAME_SEGMENTS: usize = 32;

/// Bytes of a rejected name quoted back in a validation error.
const QUOTED_NAME_BYTES: usize = 64;

/// Render a rejected name for an error message.
///
/// Only a short, control-escaped prefix is quoted: a rejected name is by
/// definition untrusted, and a line-oriented log consumer must not be able to
/// forge a record boundary from the diagnostic itself.
fn describe_name(name: &str) -> String {
    let mut quoted = String::with_capacity(QUOTED_NAME_BYTES + 2);
    quoted.push('"');
    let mut used = 0;
    for ch in name.chars() {
        let escaped = ch.escape_debug().to_string();
        if used + escaped.len() > QUOTED_NAME_BYTES {
            quoted.push('…');
            break;
        }
        used += escaped.len();
        quoted.push_str(&escaped);
    }
    quoted.push('"');
    quoted
}

/// Build the `ValueError` raised for a name the rules reject.
fn invalid_name(message: String) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyValueError, _>(message)
}

/// Validate a logger name, returning the reason it was rejected.
///
/// The checks run cheapest first, and the byte-length limit runs first of all
/// so a hostile megabyte-long name is rejected before anything scans it.
///
/// # Examples
///
/// ```rust,ignore
/// use femtologging::manager::validate_logger_name;
///
/// assert!(validate_logger_name("api.v1").is_ok(), "a component path is valid");
/// assert!(validate_logger_name("").is_err(), "an empty name has no parent chain");
/// ```
pub fn validate_logger_name(name: &str) -> PyResult<()> {
    if name.is_empty() {
        return Err(invalid_name("logger name cannot be empty".to_string()));
    }
    if name.len() > MAX_LOGGER_NAME_BYTES {
        return Err(invalid_name(format!(
            "logger name cannot exceed {MAX_LOGGER_NAME_BYTES} bytes; got {} bytes",
            name.len(),
        )));
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err(invalid_name(format!(
            "logger name cannot start or end with '.': {}",
            describe_name(name),
        )));
    }
    if name.split('.').any(str::is_empty) {
        return Err(invalid_name(format!(
            "logger name cannot contain consecutive dots: {}",
            describe_name(name),
        )));
    }
    let segments = name.split('.').count();
    if segments > MAX_LOGGER_NAME_SEGMENTS {
        return Err(invalid_name(format!(
            "logger name cannot exceed {MAX_LOGGER_NAME_SEGMENTS} dot-separated segments; got {segments}",
        )));
    }
    if let Some(control) = name.chars().find(char::is_ascii_control) {
        return Err(invalid_name(format!(
            "logger name cannot contain the ASCII control character {}: {}",
            control.escape_debug(),
            describe_name(name),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Tests for logger name validation.

    use super::{
        MAX_LOGGER_NAME_BYTES, MAX_LOGGER_NAME_SEGMENTS, describe_name, validate_logger_name,
    };

    /// The rejection message, so a test can assert on the diagnostic too.
    fn rejection(name: &str) -> String {
        validate_logger_name(name)
            .expect_err("name should be rejected")
            .to_string()
    }

    #[test]
    fn accepts_a_dotted_component_path() {
        validate_logger_name("api.v1.handlers").expect("a component path is a valid name");
        validate_logger_name("root").expect("root is a valid name");
        validate_logger_name("single").expect("a bare component is a valid name");
    }

    #[test]
    fn accepts_a_name_at_the_byte_boundary() {
        let name = "a".repeat(MAX_LOGGER_NAME_BYTES);
        validate_logger_name(&name).expect("a name exactly at the byte budget is accepted");
    }

    /// A name of exactly `segments` single-character dot-separated segments.
    fn segmented_name(segments: usize) -> String {
        vec!["s"; segments].join(".")
    }

    #[test]
    fn accepts_a_name_at_the_segment_boundary() {
        let name = segmented_name(MAX_LOGGER_NAME_SEGMENTS);
        validate_logger_name(&name).expect("a name exactly at the segment budget is accepted");
    }

    #[test]
    fn rejects_an_empty_name() {
        let message = rejection("");
        assert!(
            message.contains("empty"),
            "the diagnostic should say the name is empty, got {message}",
        );
    }

    #[test]
    fn rejects_a_name_one_byte_over_the_budget() {
        let name = "a".repeat(MAX_LOGGER_NAME_BYTES + 1);
        let message = rejection(&name);
        assert!(
            message.contains("cannot exceed"),
            "the diagnostic should name the byte budget, got {message}",
        );
        assert!(
            !message.contains(&name),
            "an over-long name must not be quoted back in full",
        );
    }

    #[test]
    fn rejects_a_name_one_segment_over_the_budget() {
        let name = segmented_name(MAX_LOGGER_NAME_SEGMENTS + 1);
        let message = rejection(&name);
        assert!(
            message.contains("segments"),
            "the diagnostic should name the segment budget, got {message}",
        );
    }

    #[test]
    fn rejects_names_with_an_undefined_parent_chain() {
        for name in [".api", "api.", ".", "api..v1"] {
            assert!(
                validate_logger_name(name).is_err(),
                "{name:?} should be rejected because its parent chain is undefined",
            );
        }
    }

    #[test]
    fn rejects_every_ascii_control_character() {
        let control_characters = (0u8..=0x1f)
            .map(char::from)
            .chain(std::iter::once('\u{7f}'));
        for control in control_characters {
            let name = format!("api{control}v1");
            let message = rejection(&name);
            assert!(
                message.contains("control character"),
                "{:#04x} should be refused as a control character, got {message}",
                u32::from(control),
            );
        }
    }

    #[test]
    fn a_rejected_name_cannot_forge_a_record_boundary() {
        // A name carrying a newline must not put a raw newline in the message
        // that reports the rejection: that diagnostic reaches the same
        // line-oriented sinks the name was refused for.
        for name in ["api\nv1", "api\r\nv1", "api\u{1b}[31mred"] {
            let message = rejection(name);
            assert!(
                !message.contains('\n') && !message.contains('\r') && !message.contains('\u{1b}'),
                "the diagnostic for {name:?} leaked a raw control character: {message:?}",
            );
            assert!(
                message.contains("control character"),
                "the diagnostic for {name:?} should explain the refusal, got {message}",
            );
        }
    }

    #[test]
    fn describe_name_truncates_long_names() {
        let name = "x".repeat(1_000);
        let quoted = describe_name(&name);
        assert!(
            quoted.len() <= 64 + "…".len() + 2,
            "the quoted prefix should stay bounded, got {} bytes",
            quoted.len(),
        );
        assert!(
            quoted.ends_with("…\""),
            "a truncated prefix should be marked as truncated, got {quoted}",
        );
    }
}
