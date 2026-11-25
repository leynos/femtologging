//! Coverage for the drop diagnostic's exact wire text.

use super::report_context_drop;
use crate::log_context;

/// The written line is the diagnostic text followed by exactly one newline.
///
/// A test that only called the formatter would pass even if the write were
/// deleted, so this drives the writer helper the logging path uses.
#[test]
fn the_drop_diagnostic_is_written_as_one_line() {
    let mut oversize = std::collections::BTreeMap::new();
    oversize.insert(String::from("oversize"), "x".repeat(1_025));
    let error = log_context::merge_context_values(&oversize)
        .expect_err("an oversize value must be refused");

    let mut written = Vec::new();
    report_context_drop(&mut written, &error).expect("a Vec accepts the write");

    let text = String::from_utf8(written).expect("the diagnostic is UTF-8");
    assert_eq!(
        text,
        format!("FemtoLogger: dropping record due to invalid context payload: {error}\n"),
        "the drop path must write the diagnostic and one newline",
    );
}
