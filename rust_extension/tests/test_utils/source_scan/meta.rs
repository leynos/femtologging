//! Judging one parsed attribute: which protected lints it suppresses, and how
//! it reads back in a failure message.
//!
//! This is the half of the scan that reasons about a `Meta`. The other half,
//! in `tokens`, reaches attributes a `Meta` never describes.

use syn::{
    AttrStyle, Attribute, Meta, MetaList, Path, Token, ext::IdentExt, punctuated::Punctuated,
};

/// Render a lint path with raw identifiers normalized.
///
/// `r#allow` is `allow` and `clippy::r#style` is `clippy::style`; Clippy
/// honours both spellings, so comparing the written form would let either
/// through. Measured: `#![r#allow(clippy::disallowed_methods)]` and
/// `#![allow(clippy::r#style)]` each reduce the probe from one diagnostic to
/// none.
pub(super) fn render_path(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.unraw().to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Return the lint names an `allow` meta-list suppresses.
///
/// Key-value arguments such as `reason = "..."` are not lint names and are
/// skipped.
fn allowed_lints(list: &MetaList) -> Vec<String> {
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return Vec::new();
    };
    nested
        .iter()
        .filter_map(|meta| match meta {
            Meta::Path(path) => Some(render_path(path)),
            Meta::List(_) | Meta::NameValue(_) => None,
        })
        .collect()
}

/// Return the lint names one attribute suppresses, following `cfg_attr`.
///
/// `inner` is the scope of the attribute this began at, and a `cfg_attr`
/// carries it down: `#![cfg_attr(all(), expect(...))]` is crate-scoped however
/// deeply the nesting runs.
///
/// `expect` is judged by that scope rather than exempted outright. An
/// item-scoped `#[expect(..., reason = "...")]` is the sanctioned form and is
/// left alone; a crate-scoped `#![expect(...)]` is not, because one call
/// anywhere in the crate fulfils it and the rest go unreported. Measured:
/// `#![expect(clippy::disallowed_methods)]` reports neither the disallowed
/// method nor an unfulfilled expectation, so nothing at all is left to notice.
pub(super) fn suppressed_by(meta: &Meta, inner: bool) -> Vec<String> {
    let Ok(list) = meta.require_list() else {
        return Vec::new();
    };
    match render_path(meta.path()).as_str() {
        "allow" => allowed_lints(list),
        "expect" if inner => allowed_lints(list),
        "cfg_attr" => suppressed_by_cfg_attr(list, inner),
        _ => Vec::new(),
    }
}

/// Return the lint names nested inside a `cfg_attr`.
///
/// The condition is followed whatever it says. A suppression that applies
/// under some configuration is still a suppression, and deciding which
/// configurations are reachable is not this contract's job.
fn suppressed_by_cfg_attr(list: &MetaList, inner: bool) -> Vec<String> {
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return Vec::new();
    };
    nested
        .iter()
        .skip(1)
        .flat_map(|meta| suppressed_by(meta, inner))
        .collect()
}

/// Render an attribute roughly as written, for a failure message.
pub(super) fn render_attribute(attribute: &Attribute) -> String {
    let bang = match attribute.style {
        AttrStyle::Inner(_) => "!",
        AttrStyle::Outer => "",
    };
    let path = render_path(attribute.path());
    attribute.meta.require_list().map_or_else(
        |_| format!("#{bang}[{path}]"),
        |list| format!("#{bang}[{path}({})]", list.tokens),
    )
}
