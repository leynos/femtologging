//! Reserved record and metadata names that structured fields must not use.
//!
//! The sets are shared by context admission, callback enrichment, and HTTP
//! serialization so every path agrees on which names are built-in fields.

/// Stdlib `logging.LogRecord` attribute names reserved for built-in fields.
const RESERVED_STD_RECORD_KEYS: &[&str] = &[
    "name",
    "msg",
    "args",
    "levelname",
    "levelno",
    "pathname",
    "filename",
    "module",
    "exc_info",
    "exc_text",
    "stack_info",
    "lineno",
    "funcName",
    "created",
    "msecs",
    "relativeCreated",
    "thread",
    "threadName",
    "process",
    "processName",
    "message",
    "asctime",
    "taskName",
];

/// Femtologging-specific record and metadata names reserved for built-in fields.
const RESERVED_FEMTO_KEYS: &[&str] = &[
    "logger",
    "level",
    "metadata",
    "module_path",
    "line_number",
    "timestamp",
    "thread_id",
    "thread_name",
    "key_values",
];

/// Report whether `key` collides with a built-in record or metadata field.
///
/// Matching is exact and case-sensitive: `"Name"` and `"logger_name"` are
/// custom fields, while `"name"` and `"logger"` are reserved.
pub(crate) fn is_reserved_structured_key(key: &str) -> bool {
    RESERVED_STD_RECORD_KEYS.contains(&key) || RESERVED_FEMTO_KEYS.contains(&key)
}

#[cfg(test)]
mod tests {
    //! Property tests for the reserved-name contract.
    //!
    //! The contract is an exact-match list rather than a prefix, substring or
    //! case-insensitive rule, so the generated cases pin the boundary of the
    //! predicate instead of repeating a handful of hand-picked names.

    use proptest::prelude::*;

    use super::{RESERVED_FEMTO_KEYS, RESERVED_STD_RECORD_KEYS, is_reserved_structured_key};

    /// Every reserved name across both sets, in contract order.
    fn reserved_names() -> impl Iterator<Item = &'static str> {
        RESERVED_STD_RECORD_KEYS
            .iter()
            .copied()
            .chain(RESERVED_FEMTO_KEYS.iter().copied())
    }

    proptest! {
        /// Only exact members of the documented sets are reported reserved.
        #[test]
        fn generated_keys_are_reserved_only_by_exact_membership(candidate in "[A-Za-z0-9_]{0,24}") {
            let expected = RESERVED_STD_RECORD_KEYS.contains(&candidate.as_str())
                || RESERVED_FEMTO_KEYS.contains(&candidate.as_str());
            prop_assert_eq!(is_reserved_structured_key(&candidate), expected);
        }

        /// Recasing a reserved name turns it into a legitimate custom field.
        #[test]
        fn case_variants_of_reserved_names_stay_custom(
            name in prop::sample::select(reserved_names().collect::<Vec<_>>()),
            uppercase in any::<bool>(),
        ) {
            let variant = if uppercase {
                name.to_uppercase()
            } else {
                name.to_lowercase()
            };
            prop_assume!(variant != name);
            prop_assert!(
                !is_reserved_structured_key(&variant),
                "{variant} should be a custom field, not the reserved name {name}",
            );
        }
    }
}
