//! Fence rules for the documentation-example loader.
//!
//! `build.rs` uses `test_utils/doc_examples.rs` to turn the published guide
//! into test sources. These cases pin the Markdown rules that decide which
//! fences are examples, so a change to the loader cannot silently stop a guide
//! example being compiled.

#[path = "test_utils/doc_examples.rs"]
mod doc_examples;

use doc_examples::extract_examples;
use proptest::prelude::*;
use rstest::rstest;

/// Extract the single example in `guide` and return the source it publishes.
///
/// The fence-closing cases differ only in the Markdown they scan, so they
/// share this helper instead of repeating the extraction and indexing.
fn only_example_source(guide: &str) -> String {
    let examples = extract_examples(guide).expect("the guide should be valid");
    assert_eq!(examples.len(), 1, "the guide should hold one example");
    examples[0].source.clone()
}

/// A marked Rust fence is extracted with its body and identifier.
#[test]
fn a_marked_rust_fence_is_extracted() {
    let guide = "<!-- tested-example: example -->\n```rust\nfn demo() {}\n```\n";
    let examples = extract_examples(guide).expect("the guide should be valid");
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0].identifier, "example");
    assert_eq!(examples[0].source, "fn demo() {}\n");
}

/// Options after the language name do not hide the fence from the loader.
#[rstest]
#[case("rust,no_run")]
#[case("rust ignore")]
#[case("rust,no_run,edition2021")]
fn info_string_options_keep_the_language(#[case] opening: &str) {
    let guide = format!("<!-- tested-example: example -->\n```{opening}\nfn demo() {{}}\n```\n");
    let examples = extract_examples(&guide).expect("the guide should be valid");
    assert_eq!(examples.len(), 1, "the fence should still be Rust");
    assert_eq!(examples[0].source, "fn demo() {}\n");
}

/// A tilde fence is a fence too, and its language still selects Rust.
#[test]
fn a_marked_tilde_fence_is_extracted() {
    let guide = "<!-- tested-example: example -->\n~~~rust\nfn demo() {}\n~~~\n";
    let examples = extract_examples(guide).expect("the guide should be valid");
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0].source, "fn demo() {}\n");
}

/// Only a longer run of the opener's character closes a fence.
///
/// The first case puts a standalone three-backtick line inside a four-backtick
/// fence, so it fails if a shorter run closes the fence. The second puts a
/// tilde line inside a backtick fence, so it fails if the closer ignores the
/// character it was opened with. In both the inner line stays in the body.
#[rstest]
#[case("````rust\nlet s = \"```\";\n```\n````\n", "let s = \"```\";\n```\n")]
#[case("```rust\nfn demo() {}\n~~~\n```\n", "fn demo() {}\n~~~\n")]
fn only_a_matching_fence_run_closes(#[case] fence: &str, #[case] expected: &str) {
    let guide = format!("<!-- tested-example: example -->\n{fence}");
    assert_eq!(only_example_source(&guide), expected);
}

/// A fence run followed by text is content, not a closing fence.
#[test]
fn a_fence_run_followed_by_text_does_not_close() {
    let guide = concat!(
        "<!-- tested-example: example -->\n",
        "```rust\n",
        "let x = 1;\n",
        "``` this line is content\n",
        "```\n",
    );
    let source = only_example_source(guide);
    assert!(source.contains("``` this line is content"));
}

/// An unmarked Rust fence is an error, whatever its info string or character.
#[rstest]
#[case("```rust")]
#[case("```rust,no_run")]
#[case("~~~rust")]
fn an_unmarked_rust_fence_is_rejected(#[case] opening: &str) {
    let guide = format!("{opening}\nfn demo() {{}}\n```\n");
    let error = extract_examples(&guide).expect_err("an unmarked fence must fail");
    assert!(
        error
            .to_string()
            .contains("not preceded by a tested-example marker"),
        "unexpected error: {error}"
    );
}

/// A Rust fence quoted inside another fence is content, not an example.
#[test]
fn a_quoted_rust_fence_inside_another_fence_is_ignored() {
    let guide = "```python\n```rust\n```\n";
    let examples = extract_examples(guide).expect("a quoted fence is not an example");
    assert!(examples.is_empty());
}

/// A marked fence without a language tag cannot be classified, so it fails.
#[test]
fn a_marked_fence_without_a_language_is_rejected() {
    let error = extract_examples("<!-- tested-example: example -->\n```\nfn demo() {}\n```\n")
        .expect_err("a fence without a language must fail");
    assert!(error.to_string().contains("has no language tag"), "{error}");
}

/// A marker must be followed immediately by a fenced code block.
#[test]
fn a_marker_must_precede_a_fence() {
    let error = extract_examples("<!-- tested-example: example -->\n\n```rust\n```\n")
        .expect_err("a marker without a fence must fail");
    assert!(
        error
            .to_string()
            .contains("not followed by a fenced code block"),
        "{error}"
    );
}

/// A marker at the end of the document has no fence to introduce.
#[test]
fn a_marker_at_the_end_of_the_document_is_rejected() {
    let error = extract_examples("<!-- tested-example: example -->\n")
        .expect_err("a trailing marker must fail");
    assert!(error.to_string().contains("unterminated"), "{error}");
}

/// A marker that does not parse as one is reported rather than skipped.
#[test]
fn a_malformed_marker_is_rejected() {
    let error = extract_examples("<!-- tested-example: example ->\n```rust\n```\n")
        .expect_err("a malformed marker must fail");
    assert!(error.to_string().contains("malformed"), "{error}");
}

/// Two examples may not claim the same identifier.
#[test]
fn duplicate_identifiers_are_rejected() {
    let guide = concat!(
        "<!-- tested-example: example -->\n```rust\n```\n",
        "<!-- tested-example: example -->\n```rust\n```\n",
    );
    let error = extract_examples(guide).expect_err("a duplicate identifier must fail");
    assert!(error.to_string().contains("duplicate"), "{error}");
}

/// An identifier that is not a plain file name is rejected.
#[rstest]
#[case("../escape")]
#[case("Upper")]
#[case("under_score")]
#[case("with space")]
#[case("with.dot")]
fn invalid_identifiers_are_rejected(#[case] identifier: &str) {
    let guide = format!("<!-- tested-example: {identifier} -->\n```rust\n```\n");
    let error = extract_examples(&guide).expect_err("an invalid identifier must fail");
    assert!(
        error
            .to_string()
            .contains("invalid tested-example identifier"),
        "{error}"
    );
}

/// A fence that is opened but never closed is an error, not a silent skip.
#[test]
fn an_unterminated_fence_is_rejected() {
    let error = extract_examples("<!-- tested-example: example -->\n```rust\nfn demo() {}\n")
        .expect_err("an unterminated fence must fail");
    assert!(error.to_string().contains("unterminated"), "{error}");
}

/// The identifier determines the generated module's path.
#[test]
fn an_identifier_maps_to_a_generated_module_name() {
    let guide =
        "<!-- tested-example: process-payload-schema-versioning -->\n```rust\nfn demo() {}\n```\n";
    let examples = extract_examples(guide).expect("the guide should be valid");
    assert_eq!(
        examples[0].output_name(),
        "tests/generated/process_payload_schema_versioning.rs"
    );
}

proptest! {
    /// Arbitrary guide text is either rejected or scanned without panicking.
    ///
    /// The loader runs inside the build script, where a panic would surface as
    /// an opaque build failure rather than a contract violation.
    #[test]
    fn extraction_never_panics(text in ".{0,400}") {
        let _ = extract_examples(&text);
    }

    /// A matched marker and Rust fence round-trip their body verbatim.
    ///
    /// The body alphabet excludes the fence character and line breaks, so the
    /// generated guide always closes the fence the case opened.
    #[test]
    fn a_matched_fence_round_trips_its_body(body in "[a-z !,.;:()]{0,120}") {
        let guide = format!("<!-- tested-example: demo -->\n```rust\n{body}\n```\n");
        let examples = extract_examples(&guide).expect("the generated guide is valid");
        let published = format!("{body}\n");
        prop_assert_eq!(examples.len(), 1);
        prop_assert_eq!(examples[0].source.as_str(), published.as_str());
    }

    /// A guide of several marked fences publishes every one, in order.
    ///
    /// The bodies avoid the fence character and line breaks, so each fence
    /// closes where the case opened it. Identifiers are distinct, so the case
    /// also covers a guide the loader accepts as a whole.
    #[test]
    fn every_marked_fence_is_published_in_document_order(
        bodies in proptest::collection::vec("[a-z !,.;:()]{0,40}", 1..6),
    ) {
        let mut guide = String::new();
        for (index, body) in bodies.iter().enumerate() {
            guide.push_str(&format!(
                "<!-- tested-example: case-{index} -->\n```rust\n{body}\n```\n"
            ));
        }

        let examples = extract_examples(&guide).expect("the generated guide is valid");

        let identifiers: Vec<String> = examples
            .iter()
            .map(|example| example.identifier.clone())
            .collect();
        let expected_identifiers: Vec<String> =
            (0..bodies.len()).map(|index| format!("case-{index}")).collect();
        prop_assert_eq!(identifiers, expected_identifiers);

        let sources: Vec<String> = examples
            .iter()
            .map(|example| example.source.clone())
            .collect();
        let expected_sources: Vec<String> = bodies.iter().map(|body| format!("{body}\n")).collect();
        prop_assert_eq!(sources, expected_sources);
    }

    /// Every accepted identifier maps to a module name that keeps it distinct.
    ///
    /// Dashes become underscores, and because the alphabet has no underscores
    /// two identifiers cannot collapse to the same file name.
    #[test]
    fn accepted_identifiers_map_to_distinct_module_names(
        first in "[a-z0-9]{1,4}(-[a-z0-9]{1,4}){0,3}",
        second in "[a-z0-9]{1,4}(-[a-z0-9]{1,4}){0,3}",
    ) {
        prop_assume!(first != second);
        let guide = format!(
            concat!(
                "<!-- tested-example: {first} -->\n```rust\nfn first() {{}}\n```\n",
                "<!-- tested-example: {second} -->\n```rust\nfn second() {{}}\n```\n",
            ),
            first = first,
            second = second,
        );
        let examples = extract_examples(&guide).expect("the generated guide is valid");
        prop_assert_ne!(examples[0].output_name(), examples[1].output_name());
    }
}
