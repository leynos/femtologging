//! Judging an `include!` target, which the scan would have to read as a second
//! source.
//!
//! `rustc` parses an included file as Rust whatever its extension, and resolves
//! the target against the file that writes it. Neither fact is visible to the
//! attribute walk: an `allow` written in the included file is a complete,
//! ordinary attribute, in a file the scan may never open. The inclusion itself
//! is therefore the finding, judged here rather than beside the token readers,
//! because what it needs is the walk's own rule for which paths are reachable.

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use proc_macro2::TokenStream;
use syn::LitStr;

use super::SOURCE_EXTENSION;
use super::discovery::is_walkable;

/// Return a finding if an `include!` names a target that is not Rust source.
///
/// `rustc` parses an included file as Rust whatever its extension, so
/// `include!("fixture.rs.txt")` compiles the fixture's contents into this
/// crate. An `allow` written there suppresses the policy for the calls around
/// it, and an enclosing `expect` stays fulfilled, so nothing warns. The scan
/// cannot read the target, because the target need not exist when the scan
/// runs, so the inclusion itself is the finding.
///
/// A literal `.rs` path is not a finding *if the walk would reach it*, since
/// such a file is then scanned in its own right. A `.rs` extension alone is
/// not enough: `rustc` resolves the target against the file that writes it,
/// and the walk skips `target` and every dot-prefixed directory, so
/// `include!(".generated/bypass.rs")` names a real Rust file the compiler
/// reads and the scan never opens. The target is resolved against the
/// including source and every directory it passes through is judged by
/// [`is_walkable`], the same function the walk descends with, so the two
/// cannot drift. A target that climbs out of the crate is refused for the same
/// reason. Only the directories are judged, because the walk collects a file by
/// its extension alone and reads a dot-prefixed *file* like any other.
///
/// `include_str!` and `include_bytes!` are not source inclusion at all and
/// never reach here.
///
/// The target is parsed as one [`LitStr`] and judged by its *value*, not by
/// how it was written. `r"support.rs"` and `"support\x2Ers"` name the same
/// file as `"support.rs"`, and rendering the literal back to text would report
/// two of the three as targets the scan cannot see. Parsing the whole token
/// stream as a single literal is also what keeps a computed target refused:
/// `concat!(env!("OUT_DIR"), "/probe")` is not one literal and does not parse.
///
/// The extension is compared the way the traversal selects sources, against
/// [`SOURCE_EXTENSION`], rather than by a suffix test on the rendered path. A
/// suffix test is case-sensitive in a way the path reader is not, and it
/// accepts `include!(".rs")`, a name that is a bare extension and that the walk
/// never collects, so the file would go unread and unscanned.
pub(super) fn foreign_inclusion(tokens: &TokenStream, including: &Utf8Path) -> Option<String> {
    let Ok(target) = syn::parse2::<LitStr>(tokens.clone()) else {
        let rendered = tokens.to_string();
        return Some(format!(
            "include!({rendered}) names a target the scan cannot resolve; \
             name a literal `.rs` path, which is scanned in its own right"
        ));
    };
    let path = target.value();
    if Utf8Path::new(&path).extension() != Some(SOURCE_EXTENSION) {
        return Some(format!(
            "include!(\"{path}\") compiles a file the scan cannot see as Rust; \
             name a `.rs` path, which is scanned in its own right"
        ));
    }
    let Some(resolved) = resolve_against(including, Utf8Path::new(&path)) else {
        return Some(format!(
            "include!(\"{path}\") from {including} resolves outside the crate the \
             walk reads; name a `.rs` path inside it, which is scanned in its own right"
        ));
    };
    let skipped = resolved
        .components()
        .rev()
        .skip(1)
        .find_map(|component| match component {
            Utf8Component::Normal(name) if !is_walkable(name) => Some(name.to_owned()),
            _ => None,
        })?;
    Some(format!(
        "include!(\"{path}\") from {including} resolves to {resolved}, under \
         `{skipped}`, which the walk skips; name a `.rs` path the walk collects, \
         which is scanned in its own right"
    ))
}

/// Resolve `target` against the directory holding `including`, lexically.
///
/// Lexical rather than filesystem resolution, because the scan has to judge a
/// target that need not exist when it runs, and because a walk that followed
/// links or canonicalised paths could be led outside the tree it was handed.
///
/// `None` means the target climbs above the crate directory, or names an
/// absolute path: either way no walk rooted there reaches it.
fn resolve_against(including: &Utf8Path, target: &Utf8Path) -> Option<Utf8PathBuf> {
    let base = including.parent().unwrap_or_else(|| Utf8Path::new(""));
    let mut parts: Vec<&str> = Vec::new();
    for component in base.components().chain(target.components()) {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                parts.pop()?;
            }
            Utf8Component::Normal(name) => parts.push(name),
            Utf8Component::RootDir | Utf8Component::Prefix(_) => return None,
        }
    }
    Some(parts.iter().collect())
}
