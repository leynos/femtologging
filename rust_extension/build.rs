//! Load the Rust examples published in the user guide into the test tree.
//!
//! `docs/documentation-style-guide.md` requires user-facing examples to be
//! exercised as they are published, rather than as a copy kept beside them.
//! A build script is the earliest point at which the published text can
//! become Rust source: it extracts every marked `tested-example` fence from
//! `docs/users-guide.md` and writes it under `tests/generated`. An integration
//! test then `include!`s the generated file, so the test compiles exactly the
//! text the guide ships and the two cannot drift apart.
//!
//! The generated files are written inside `tests` rather than to `OUT_DIR`
//! because the environment-policy scan in `tests/env_policy_source_scan.rs`
//! refuses any `include!` target the walk never reads. A file under build
//! output could carry a lint suppression the scan would never see, and that
//! refusal is deliberately not weakened for documentation. Generated files
//! are ignored by Git; regenerate them by building the crate.
//!
//! The loader fails the build when a Rust fence is unmarked, when a marker is
//! malformed, unnamed, or repeated, and when a fence is unterminated. A
//! silently missing example would leave the guide untested, so these are
//! errors rather than warnings.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io;

use camino::Utf8Path;
use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;

/// Marker that introduces a tested example.
const MARKER_PREFIX: &str = "<!-- tested-example: ";

/// Marker that closes a tested-example identifier.
const MARKER_SUFFIX: &str = " -->";

/// Text that distinguishes a tested-example marker from ordinary prose.
const MARKER_HINT: &str = "tested-example:";

/// Prefix shared by every fenced code block.
const FENCE_OPEN: &str = "```";

/// Opening line of a Rust fence.
const RUST_FENCE: &str = "```rust";

/// Line that closes any fence.
const FENCE_END: &str = "```";

/// Directory, relative to the crate root, that receives the generated modules.
const GENERATED_DIR: &str = "tests/generated";

/// Suffix of the in-progress file that is renamed into place.
const TEMP_SUFFIX: &str = ".tmp";

/// Extension of a generated module.
const GENERATED_EXTENSION: &str = ".rs";

/// Guide path, relative to the repository root one level above the crate.
const GUIDE_PATH: &str = "docs/users-guide.md";

/// One example loaded from the guide.
struct Example {
    /// Stable identifier taken from the marker, with dashes preserved.
    identifier: String,
    /// The published Rust source, without the fence lines.
    source: String,
}

impl Example {
    /// Return the identifier in a form a Rust file name can use.
    fn file_stem(&self) -> String {
        self.identifier.replace('-', "_")
    }

    /// Return the generated module's name, relative to the crate root.
    fn output_name(&self) -> String {
        format!("{GENERATED_DIR}/{}{GENERATED_EXTENSION}", self.file_stem())
    }

    /// Return the name of the file this example is written to first.
    ///
    /// The module is renamed into place once it is complete, so a build that
    /// runs alongside another one never exposes a half-written example.
    fn temp_name(&self) -> String {
        format!("{}{TEMP_SUFFIX}", self.output_name())
    }
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
struct ContractError(String);

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

/// Extract the published examples and write them where tests can include them.
fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=../{GUIDE_PATH}");

    let repository = Dir::open_ambient_dir(manifest_dir.join(".."), ambient_authority())?;
    let text = repository.read_to_string(GUIDE_PATH)?;
    let examples = extract_examples(&text)?;

    let crate_dir = Dir::open_ambient_dir(manifest_dir, ambient_authority())?;
    crate_dir.create_dir_all(GENERATED_DIR)?;
    publish(&crate_dir, &examples)?;
    prune(&crate_dir, &examples)?;

    Ok(())
}

/// Write each example, then remove generated modules it no longer produces.
fn publish(crate_dir: &Dir, examples: &[Example]) -> io::Result<()> {
    for example in examples {
        crate_dir.write(example.temp_name(), &example.source)?;
        crate_dir.rename(example.temp_name(), crate_dir, example.output_name())?;
    }
    Ok(())
}

/// Remove generated modules left behind by an identifier that has gone.
fn prune(crate_dir: &Dir, examples: &[Example]) -> io::Result<()> {
    let expected: BTreeSet<String> = examples.iter().map(Example::output_name).collect();
    for entry in crate_dir.read_dir(GENERATED_DIR)? {
        let name = format!("{GENERATED_DIR}/{}", entry?.file_name()?);
        if name.ends_with(GENERATED_EXTENSION) && !expected.contains(&name) {
            crate_dir.remove_file(&name)?;
        }
    }
    Ok(())
}

/// Extract every marked Rust fence, rejecting violations of the contract.
fn extract_examples(guide: &str) -> Result<Vec<Example>, ContractError> {
    let lines: Vec<&str> = guide.lines().collect();
    let mut examples = Examples::default();
    let mut index = 0;

    while index < lines.len() {
        if let Some(identifier) = marker_identifier(lines[index]) {
            index = examples.read_marked_fence(&lines, index, identifier)?;
            continue;
        }
        if lines[index].contains(MARKER_HINT) {
            return Err(ContractError::new(format!(
                "malformed tested-example marker at line {}: {:?}",
                index + 1,
                lines[index]
            )));
        }
        if lines[index].trim() == RUST_FENCE {
            return Err(ContractError::new(format!(
                "Rust fence at line {} is not preceded by a tested-example marker",
                index + 1
            )));
        }
        index += 1;
    }

    Ok(examples.items)
}

impl Examples {
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
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Err(ContractError::new(format!(
                "empty tested-example identifier at line {}",
                marker_index + 1
            )));
        }
        if !self.seen.insert(identifier.to_owned()) {
            return Err(ContractError::new(format!(
                "duplicate tested-example identifier {identifier:?} at line {}",
                marker_index + 1
            )));
        }

        let fence_index = marker_index + 1;
        let opening = lines.get(fence_index).ok_or_else(|| {
            ContractError::new(format!(
                "the marker at line {} is unterminated",
                marker_index + 1
            ))
        })?;
        let language = opening.trim().strip_prefix(FENCE_OPEN).ok_or_else(|| {
            ContractError::new(format!(
                "the marker at line {} is not followed by a fenced code block",
                marker_index + 1
            ))
        })?;
        if language.is_empty() {
            return Err(ContractError::new(format!(
                "the fence after the marker at line {} has no language tag",
                marker_index + 1
            )));
        }

        let mut source = String::new();
        for (offset, line) in lines[fence_index + 1..].iter().enumerate() {
            if line.trim() == FENCE_END {
                if language == "rust" {
                    self.items.push(Example {
                        identifier: identifier.to_owned(),
                        source,
                    });
                }
                return Ok(fence_index + 1 + offset + 1);
            }
            source.push_str(line);
            source.push('\n');
        }

        Err(ContractError::new(format!(
            "the fence for {identifier:?} is unterminated"
        )))
    }
}

/// Return the identifier when the line is a tested-example marker.
fn marker_identifier(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix(MARKER_PREFIX)?;
    let identifier = rest.strip_suffix(MARKER_SUFFIX)?;
    Some(identifier.trim())
}
