//! Judging a target the scan would have to read as a second source.
//!
//! Two routes name one. `include!("target")` splices a file's contents in, and
//! `#[path = "target"] mod m;` compiles a file the module system would
//! otherwise have found by name. In both, `rustc` parses the target as Rust
//! whatever its extension and resolves it against the file that writes it.
//! Neither fact is visible to the attribute walk: an `allow` written in the
//! target is a complete, ordinary attribute, in a file the scan may never
//! open. The reference itself is therefore the finding, judged here rather
//! than beside the token readers, because what it needs is the walk's own rule
//! for which paths are reachable.
//!
//! The two routes share [`unreadable_target`] rather than each carrying a
//! copy of the rule. They are the same question about the same walk, and an
//! `include!` rule that drifted from a `#[path]` rule would close one route
//! and leave its twin open, which is how `#[path]` came to be missing in the
//! first place.

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use proc_macro2::TokenStream;
use syn::{Expr, ExprLit, Lit, LitStr, Meta, Token, punctuated::Punctuated};

use super::SOURCE_EXTENSION;
use super::discovery::is_walkable;
use super::meta::render_path;

/// Return why a target compiled as Rust is one the walk never reads.
///
/// `None` means the walk collects the target, so it is scanned in its own
/// right and the reference to it is not a finding.
///
/// A `.rs` extension alone is not enough: `rustc` resolves the target against
/// the file that writes it, and the walk skips `target` and every
/// dot-prefixed directory, so `.generated/bypass.rs` names a real Rust file
/// the compiler reads and the scan never opens. The target is resolved against
/// the referring source and every directory it passes through is judged by
/// [`is_walkable`], the same function the walk descends with, so the two
/// cannot drift. A target that climbs out of the crate is refused for the same
/// reason. Only the directories are judged, because the walk collects a file
/// by its extension alone and reads a dot-prefixed *file* like any other.
///
/// The extension is compared the way the traversal selects sources, against
/// [`SOURCE_EXTENSION`], rather than by a suffix test on the rendered path. A
/// suffix test is case-sensitive in a way the path reader is not, and it
/// accepts a bare `.rs`, a name that is all extension and that the walk never
/// collects, so the file would go unread and unscanned.
fn unreadable_target(path: &str, referring: &Utf8Path) -> Option<String> {
    if Utf8Path::new(path).extension() != Some(SOURCE_EXTENSION) {
        return Some(format!(
            "{path:?} is compiled as Rust but is not a source file the walk collects"
        ));
    }
    let Some(resolved) = resolve_against(referring, Utf8Path::new(path)) else {
        return Some(format!(
            "{path:?} resolves outside the crate the walk reads"
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
        "{path:?} resolves to {resolved}, under `{skipped}`, which the walk skips"
    ))
}

/// Return a finding if an `include!` names a target the scan cannot read.
///
/// The target is parsed as one [`LitStr`] and judged by its *value*, not by
/// how it was written. `r"support.rs"` and `"support\x2Ers"` name the same
/// file as `"support.rs"`, and rendering the literal back to text would report
/// two of the three as targets the scan cannot see. Parsing the whole token
/// stream as a single literal is also what keeps a computed target refused:
/// `concat!(env!("OUT_DIR"), "/probe")` is not one literal and does not parse.
///
/// `include_str!` and `include_bytes!` are not source inclusion at all and
/// never reach here.
pub(super) fn foreign_inclusion(tokens: &TokenStream, including: &Utf8Path) -> Option<String> {
    let Ok(target) = syn::parse2::<LitStr>(tokens.clone()) else {
        let rendered = tokens.to_string();
        return Some(format!(
            "include!({rendered}) names a target the scan cannot resolve; \
             name a literal `.rs` path, which is scanned in its own right"
        ));
    };
    let reason = unreadable_target(&target.value(), including)?;
    Some(format!(
        "include! in {including}: {reason}; name a `.rs` path the walk \
         collects, which is scanned in its own right"
    ))
}

/// Return a finding if a `#[path]` attribute names a module source the scan
/// cannot read.
///
/// `#[path = "../../bypass.rs"] mod bypass;` compiles a file outside the crate
/// directory, and `crate_sources` walks only below it, so a crate-level `allow`
/// written in that file switches the policy off for everything it covers and
/// the scan never opens it. The attribute is the finding for exactly the reason
/// an `include!` is: it names source the compiler reads and the walk does not.
///
/// The rule is the walk's own, so the idiom this crate itself uses is left
/// alone. `#[path = "source_scan/discovery.rs"]` in `tests/test_utils` names a
/// `.rs` file under a walkable directory, which the scan reads in its own
/// right, and it reports nothing.
///
/// A `path` reached through `cfg_attr` is judged too. The condition is
/// followed whatever it says, for the same reason a `cfg_attr`-wrapped `allow`
/// is: a reference that applies under some configuration still compiles a file
/// the scan never reads, and deciding which configurations are reachable is
/// not this contract's job.
///
/// The attribute's path is compared after normalizing raw identifiers, since
/// `#[r#path = "..."]` names the same attribute.
pub(super) fn foreign_module_path(meta: &Meta, referring: &Utf8Path) -> Option<String> {
    match meta {
        Meta::NameValue(pair) if render_path(&pair.path) == "path" => {
            let Expr::Lit(ExprLit {
                lit: Lit::Str(target),
                ..
            }) = &pair.value
            else {
                return None;
            };
            let reason = unreadable_target(&target.value(), referring)?;
            Some(format!(
                "#[path] in {referring}: {reason}; the module's source is \
                 compiled into this crate and never scanned"
            ))
        }
        Meta::List(list) if render_path(&list.path) == "cfg_attr" => {
            let nested = list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            nested
                .iter()
                .skip(1)
                .find_map(|inner| foreign_module_path(inner, referring))
        }
        _ => None,
    }
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
