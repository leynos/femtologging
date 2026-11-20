//! Tests for percent-style record formatting.

use std::{
    collections::BTreeMap,
    time::{Duration, UNIX_EPOCH},
};

use proptest::prelude::*;
use rstest::rstest;

use crate::{
    exception_schema::{ExceptionPayload, StackFrame, StackTracePayload},
    formatter::{FemtoFormatter, format_exception_payload, format_stack_payload},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};

use super::{
    PercentFormatter,
    directives::{ClosingParenthesisSearch, Conversion, Segment, compile},
};

/// Render `format` for a fresh info-level record.
fn render(format: &str) -> String {
    let record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");
    PercentFormatter::new(format, None).format(&record)
}

#[test]
fn formats_standard_and_structured_fields() {
    let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "inside");
    record.metadata_mut().timestamp = UNIX_EPOCH + Duration::from_secs(0);
    record.metadata_mut().key_values =
        BTreeMap::from([("correlation_id".to_owned(), "abc123".to_owned())]);

    let output = PercentFormatter::new(
        "%(name)s %(levelname)s %(levelno)d %(message)s %(correlation_id)s %(missing)s %(asctime)s",
        Some("%Y".to_owned()),
    )
    .format(&record);

    assert_eq!(output, "probe INFO 2 inside abc123  1970");
}

#[test]
fn falls_back_to_default_date_format_for_invalid_datefmt() {
    let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "inside");
    record.metadata_mut().timestamp = UNIX_EPOCH + Duration::from_secs(0);

    let output = PercentFormatter::new("%(asctime)s", Some("%Q".to_owned())).format(&record);

    assert_eq!(output, "1970-01-01 00:00:00");
}

#[test]
fn formats_float_with_six_fractional_digits_by_default() {
    let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "inside");
    record.metadata_mut().key_values = BTreeMap::from([("ratio".to_owned(), "1.5".to_owned())]);

    let output = PercentFormatter::new("%(ratio)f", None).format(&record);

    assert_eq!(output, "1.500000");
}

#[test]
fn renders_missing_and_invalid_numeric_fields_as_empty() {
    let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "inside");
    record.metadata_mut().key_values = BTreeMap::from([
        ("not_an_integer".to_owned(), "oops".to_owned()),
        ("not_a_float".to_owned(), "oops".to_owned()),
    ]);

    let output = PercentFormatter::new(
        "%(not_an_integer)d|%(not_a_float)f|%(missing)d|%(missing)f",
        None,
    )
    .format(&record);

    assert_eq!(output, "|||");
}

#[test]
fn compiles_literals_and_directives_once() {
    assert_eq!(
        compile("v=%(levelno)d 100%%"),
        vec![
            Segment::Literal("v=".to_owned()),
            Segment::Directive {
                field: "levelno".to_owned(),
                conversion: Conversion::Integer,
            },
            Segment::Literal(" 100%".to_owned()),
        ]
    );
}

/// Unsupported starts are copied up to their `%`, then scanning resumes.
#[rstest]
#[case::missing_conversion_character("%(message)", "%(message)")]
#[case::missing_opening_parenthesis("%x %(message)s", "%x hello")]
#[case::non_ascii_conversion("%(message)é %(message)s", "%(message)é hello")]
#[case::unsupported_conversion("%(nope)q %(message)s", "%(nope)q hello")]
#[case::escape_inside_rejected_directive("%(bad%%)q", "%(bad%)q")]
fn continues_after_unsupported_starts(#[case] format: &str, #[case] expected: &str) {
    assert_eq!(render(format), expected);
}

#[test]
fn preserves_repeated_malformed_starts_literally() {
    // Every start lacks a closing parenthesis *or* carries a bad conversion,
    // so each `%` must be copied and the following text re-examined.
    assert_eq!(render("%(%(%("), "%(%(%(");
    assert_eq!(render("%(a)q%(b)q%(c)q"), "%(a)q%(b)q%(c)q");
}

#[test]
fn renders_literal_percent_signs() {
    assert_eq!(render("%%"), "%");
    assert_eq!(render("100%% sure"), "100% sure");
    assert_eq!(render("%%%"), "%%");
    assert_eq!(render("trailing%"), "trailing%");
}

#[test]
fn formats_repeated_records_from_one_compiled_formatter() {
    let formatter = PercentFormatter::new("%(levelname)s %(levelno)d %(message)s", None);
    let info = FemtoLogRecord::new("probe", FemtoLevel::Info, "first");
    let error = FemtoLogRecord::new("probe", FemtoLevel::Error, "second");

    assert_eq!(formatter.format(&info), "INFO 2 first");
    assert_eq!(formatter.format(&error), "ERROR 4 second");
    assert_eq!(formatter.format(&info), "INFO 2 first");
}

#[test]
fn closing_parenthesis_search_reuses_an_earlier_scan() {
    let mut search = ClosingParenthesisSearch::default();

    assert_eq!(search.find("(ab)c)", 0), Some(3));
    assert!(search.covers(0), "the searched suffix must be reused");
    assert!(
        search.covers(3),
        "the remembered `)` answers its own offset"
    );
    assert!(
        !search.covers(4),
        "offsets past the remembered `)` need a fresh scan"
    );
}

#[test]
fn closing_parenthesis_search_remembers_a_missing_delimiter() {
    let mut search = ClosingParenthesisSearch::default();

    assert_eq!(search.find("(abc", 1), None);
    assert!(search.covers(1), "the searched suffix must be reused");
    assert!(
        search.covers(usize::MAX),
        "no later suffix can contain a `)` when the searched one has none"
    );
    assert!(!search.covers(0), "an earlier offset was not searched");
}

#[test]
fn appends_stack_and_exception_payloads_after_formatted_text() {
    let exception = ExceptionPayload::new("ValueError", "invalid value");
    let stack = StackTracePayload::new(vec![StackFrame::new("app.py", 12, "run")]);
    let record = FemtoLogRecord::new("probe", FemtoLevel::Error, "failed")
        .with_stack(stack.clone())
        .with_exception(exception.clone());

    let output = PercentFormatter::new("%(message)s", None).format(&record);
    let expected = format!(
        "failed\n{}\n{}",
        format_stack_payload(&stack),
        format_exception_payload(&exception),
    );

    assert_eq!(output, expected);
}

/// Split `input` into a directive's field, conversion, and remaining suffix.
fn reference_directive(input: &str) -> Option<(&str, Conversion, &str)> {
    let field = input.strip_prefix('(')?;
    let (name, conversion_and_suffix) = field.split_once(')')?;
    let character = conversion_and_suffix.chars().next()?;
    let conversion = Conversion::from_char(character)?;
    Some((
        name,
        conversion,
        &conversion_and_suffix[character.len_utf8()..],
    ))
}

/// Render `format` with the per-record parser the formatter used before
/// compilation, so compiled output can be compared against it.
fn reference_format(formatter: &PercentFormatter, format: &str, record: &FemtoLogRecord) -> String {
    let mut output = String::new();
    let mut remaining = format;

    while let Some(index) = remaining.find('%') {
        output.push_str(&remaining[..index]);
        remaining = &remaining[index + 1..];

        if let Some(rest) = remaining.strip_prefix('%') {
            output.push('%');
            remaining = rest;
            continue;
        }

        let Some((field, conversion, suffix)) = reference_directive(remaining) else {
            output.push('%');
            continue;
        };

        output.push_str(&conversion.render(formatter.field_value(record, field)));
        remaining = suffix;
    }

    output.push_str(remaining);
    output
}

proptest! {
    #[test]
    fn formats_generated_tokens_against_reference_output(
        tokens in prop::collection::vec(prop_oneof![
            "[a-zA-Z0-9 _-]{0,8}".prop_map(|literal| (literal.clone(), literal)),
            Just(("%(message)s".to_owned(), "hello".to_owned())),
            Just(("%(levelno)d".to_owned(), "2".to_owned())),
            Just(("%(ratio)f".to_owned(), "1.500000".to_owned())),
            Just(("%(missing)s".to_owned(), String::new())),
            Just(("%%".to_owned(), "%".to_owned())),
        ], 0..16),
        malformed in prop_oneof![
            Just("%".to_owned()),
            Just("%(field".to_owned()),
            Just("%(message)q".to_owned()),
        ]
    ) {
        let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");
        record.metadata_mut().key_values = BTreeMap::from([
            ("ratio".to_owned(), "1.5".to_owned()),
        ]);
        let format = tokens
            .iter()
            .map(|(format, _)| format.as_str())
            .collect::<Vec<_>>()
            .join("|")
            + "|"
            + &malformed;
        let expected = tokens
            .iter()
            .map(|(_, output)| output.as_str())
            .collect::<Vec<_>>()
            .join("|")
            + "|"
            + &malformed;

        prop_assert_eq!(PercentFormatter::new(format, None).format(&record), expected);
    }

    #[test]
    fn compiled_output_matches_the_reference_parser(
        format in prop::collection::vec(any::<char>(), 0..128)
            .prop_map(|characters| characters.into_iter().collect::<String>())
    ) {
        let mut record = FemtoLogRecord::new("probe", FemtoLevel::Info, "hello");
        record.metadata_mut().key_values = BTreeMap::from([
            ("ratio".to_owned(), "1.5".to_owned()),
            ("count".to_owned(), "7".to_owned()),
        ]);
        let formatter = PercentFormatter::new(format.clone(), None);

        prop_assert_eq!(
            formatter.format(&record),
            reference_format(&formatter, &format, &record),
        );
    }
}
