//! Extract the `tested-example` fences published in the user guide.
//!
//! `docs/documentation-style-guide.md` requires user-facing examples to be
//! exercised as they are published. This module turns the guide's Markdown
//! into Rust source. Two callers share it: `build.rs` writes the generated
//! modules into `tests/generated` so the integration tests compile the shipped
//! text, and `tests/doc_example_loader.rs` exercises the fence rules directly.
//! It performs no I/O and reads no environment, which is what lets a build
//! script and a test binary both use it.
//!
//! The loader fails on an unmarked Rust fence, a malformed, unnamed, repeated,
//! or invalid marker, and an unterminated fence. A silently missing example
//! would leave the guide untested, so these are errors rather than warnings.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

/// Marker that introduces a tested example.
const MARKER_PREFIX: &str = "<!-- tested-example: ";

/// Marker that closes a tested-example identifier.
const MARKER_SUFFIX: &str = " -->";

/// Text that distinguishes a tested-example marker from ordinary prose.
const MARKER_HINT: &str = "tested-example:";

/// Directory, relative to the crate root, that receives the generated modules.
pub const GENERATED_DIR: &str = "tests/generated";

/// Extension of a generated module.
const GENERATED_EXTENSION: &str = ".rs";

/// Smallest run of fence characters that opens or closes a fence.
const MIN_FENCE_LENGTH: usize = 3;

/// Info-string language whose fence body is Rust source.
const RUST_LANGUAGE: &str = "rust";

/// Characters a fence may be drawn with.
const FENCE_CHARACTERS: [char; 2] = ['`', '~'];

/// One example loaded from the guide.
#[derive(Debug)]
pub struct Example {
    /// Stable identifier taken from the marker, with dashes preserved.
    pub identifier: String,
    /// The published Rust source, without the fence lines.
    pub source: String,
}

impl Example {
    /// Return the identifier in a form a Rust file name can use.
    ///
    /// Identifiers may only contain lowercase letters, digits, and dashes, so
    /// replacing each dash with an underscore cannot make two identifiers
    /// collide.
    pub fn file_stem(&self) -> String {
        self.identifier.replace('-', "_")
    }

    /// Return the generated module's name, relative to the crate root.
    pub fn output_name(&self) -> String {
        format!("{GENERATED_DIR}/{}{GENERATED_EXTENSION}", self.file_stem())
    }
}

/// A fenced code block, described by its opening line.
#[derive(Debug)]
struct Fence {
    /// The character the fence is drawn with, backtick or tilde.
    character: char,
    /// The number of fence characters in the opening run.
    length: usize,
    /// The first token of the info string, such as `rust` in `rust,no_run`.
    language: String,
}

impl Fence {
    /// Parse a fence's opening line, when the line is one.
    ///
    /// A fence opens with a run of at least three backticks or tildes. The
    /// info string may carry options after the language, separated by commas
    /// or whitespace, and CommonMark ignores those options when naming the
    /// language.
    fn open(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        let character = trimmed
            .chars()
            .next()
            .filter(|candidate| FENCE_CHARACTERS.contains(candidate))?;
        let length = trimmed.chars().take_while(|c| *c == character).count();
        if length < MIN_FENCE_LENGTH {
            return None;
        }
        // The fence characters are ASCII, so the character count is also the
        // byte offset where the info string starts.
        let info = trimmed[length..].trim_start();
        Some(Self {
            character,
            length,
            language: first_token(info).to_owned(),
        })
    }

    /// Return whether `line` closes this fence.
    ///
    /// A closing fence is drawn with the same character as the opener and is
    /// at least as long, which is what lets a longer fence contain a shorter
    /// one. Nothing may follow it but whitespace, so a line that starts with
    /// a fence run and continues with text is content, not a terminator.
    fn closes(&self, line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.chars().count() >= self.length && trimmed.chars().all(|c| c == self.character)
    }
}

/// Return the first token of an info string, splitting on comma or whitespace.
fn first_token(info: &str) -> &str {
    info.split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or_default()
}

/// The examples discovered while scanning the guide.
#[derive(Default)]
struct Examples {
    /// Generated examples, in document order.
    items: Vec<Example>,
    /// Identifiers already seen, used to reject duplicates.
    seen: BTreeSet<String>,
}

/// A violation of the documentation loader contract.
#[derive(Debug)]
pub struct ContractError(String);

impl ContractError {
    /// Describe the contract violation.
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ContractError {}

/// Extract every marked Rust fence, rejecting violations of the contract.
///
/// Fences of other languages are skipped whole, so a Rust fence quoted inside
/// a documentation example is not mistaken for an example of its own.
pub fn extract_examples(guide: &str) -> Result<Vec<Example>, ContractError> {
    let lines: Vec<&str> = guide.lines().collect();
    let mut examples = Examples::default();
    let mut index = 0;

    while let Some(line) = lines.get(index) {
        index = examples.read_line(&lines, index, line)?;
    }

    Ok(examples.items)
}

impl Examples {
    /// Handle one line and return the index of the next line to inspect.
    ///
    /// The loop in [`extract_examples`] relies on every arm advancing the
    /// index, so a line is never reconsidered.
    fn read_line(
        &mut self,
        lines: &[&str],
        index: usize,
        line: &str,
    ) -> Result<usize, ContractError> {
        if let Some(identifier) = marker_identifier(line) {
            return self.read_marked_fence(lines, index, identifier);
        }
        if line.contains(MARKER_HINT) {
            return Err(ContractError::new(format!(
                "malformed tested-example marker at line {}: {line:?}",
                index + 1
            )));
        }
        match Fence::open(line) {
            Some(fence) => skip_unmarked_fence(lines, index, &fence),
            None => Ok(index + 1),
        }
    }

    /// Read the fence that follows a marker and return the next line index.
    ///
    /// Every marked fence is validated, and Rust fences are collected for
    /// generation. The returned index is the first line after the terminator.
    fn read_marked_fence(
        &mut self,
        lines: &[&str],
        marker_index: usize,
        identifier: &str,
    ) -> Result<usize, ContractError> {
        let identifier = validate_identifier(identifier, marker_index)?;
        if !self.seen.insert(identifier.to_owned()) {
            return Err(ContractError::new(format!(
                "duplicate tested-example identifier {identifier:?} at line {}",
                marker_index + 1
            )));
        }

        let fence_index = marker_index + 1;
        let Some(opening_line) = lines.get(fence_index) else {
            return Err(ContractError::new(format!(
                "the marker at line {} is unterminated",
                marker_index + 1
            )));
        };
        let Some(fence) = Fence::open(opening_line) else {
            return Err(ContractError::new(format!(
                "the marker at line {} is not followed by a fenced code block",
                marker_index + 1
            )));
        };
        if fence.language.is_empty() {
            return Err(ContractError::new(format!(
                "the fence after the marker at line {} has no language tag",
                marker_index + 1
            )));
        }

        let (source, next) = read_body(lines, fence_index, &fence)?;
        if fence.language == RUST_LANGUAGE {
            self.items.push(Example {
                identifier: identifier.to_owned(),
                source,
            });
        }
        Ok(next)
    }
}

/// Check a marker's identifier and return it without surrounding space.
///
/// The charset also keeps the identifier a safe file name: it cannot climb out
/// of `GENERATED_DIR` or name a directory of its own.
fn validate_identifier(identifier: &str, marker_index: usize) -> Result<&str, ContractError> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err(ContractError::new(format!(
            "empty tested-example identifier at line {}",
            marker_index + 1
        )));
    }
    let valid = identifier
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(ContractError::new(format!(
            "invalid tested-example identifier {identifier:?} at line {}: \
             use lowercase letters, digits, and dashes only",
            marker_index + 1
        )));
    }
    Ok(identifier)
}

/// Skip a fence that carries no marker, rejecting an unmarked Rust one.
fn skip_unmarked_fence(
    lines: &[&str],
    index: usize,
    fence: &Fence,
) -> Result<usize, ContractError> {
    if fence.language == RUST_LANGUAGE {
        return Err(ContractError::new(format!(
            "Rust fence at line {} is not preceded by a tested-example marker",
            index + 1
        )));
    }
    let (_, next) = read_body(lines, index, fence)?;
    Ok(next)
}

/// Return the body of the fence opened at `fence_index` and the line after it.
fn read_body(
    lines: &[&str],
    fence_index: usize,
    fence: &Fence,
) -> Result<(String, usize), ContractError> {
    let mut source = String::new();
    for (offset, line) in lines[fence_index + 1..].iter().enumerate() {
        if fence.closes(line) {
            return Ok((source, fence_index + 1 + offset + 1));
        }
        source.push_str(line);
        source.push('\n');
    }
    Err(ContractError::new(format!(
        "the fence opened at line {} is unterminated",
        fence_index + 1
    )))
}

/// Return the identifier when the line is a tested-example marker.
fn marker_identifier(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix(MARKER_PREFIX)?;
    let identifier = rest.strip_suffix(MARKER_SUFFIX)?;
    Some(identifier.trim())
}
