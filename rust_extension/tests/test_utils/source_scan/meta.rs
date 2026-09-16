//! Judging one parsed attribute: which protected lints it suppresses, and how
//! it reads back in a failure message.
//!
//! This is the half of the scan that reasons about a `Meta`. The other half,
//! in `tokens`, reaches attributes a `Meta` never describes.

use syn::{
    AttrStyle, Attribute, Meta, MetaList, Path, Token, ext::IdentExt, punctuated::Punctuated,
};

use super::PROTECTED_LINTS;

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
///
/// An argument list that does not parse reports every protected lint rather
/// than none. `#[allow($lint)]` in a `macro_rules!` transcriber is a list
/// `syn` accepts and whose contents it cannot read, so an earlier draft
/// returning an empty list exempted it silently. Invoked as
/// `suppress!(clippy::disallowed_methods)`, that arm expands to a real
/// suppression over a real `std::env::var` call, and the scan reported
/// nothing. Failing closed costs a caller who writes an unreadable argument
/// an explanation; failing open costs the policy.
fn allowed_lints(list: &MetaList) -> Vec<String> {
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return PROTECTED_LINTS
            .iter()
            .map(|lint| (*lint).to_owned())
            .collect();
    };
    nested
        .iter()
        .filter_map(|meta| match meta {
            Meta::Path(path) => Some(render_path(path)),
            Meta::List(_) | Meta::NameValue(_) => None,
        })
        .collect()
}

/// Return whether a meta-list carries a non-empty `reason = "..."`.
///
/// The sanctioned form is an item-scoped `expect` that says why the site has
/// no seam. An `expect` with no reason, or with an empty one, says nothing and
/// is judged as the suppression it is.
fn has_a_reason(list: &MetaList) -> bool {
    let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return false;
    };
    nested.iter().any(|meta| match meta {
        Meta::NameValue(pair) if render_path(&pair.path) == "reason" => match &pair.value {
            syn::Expr::Lit(literal) => match &literal.lit {
                syn::Lit::Str(text) => !text.value().trim().is_empty(),
                _ => false,
            },
            _ => false,
        },
        _ => false,
    })
}

/// Return the lint names one attribute suppresses, following `cfg_attr`.
///
/// `inner` is the scope of the attribute this began at, and a `cfg_attr`
/// carries it down: `#![cfg_attr(all(), expect(...))]` is crate-scoped however
/// deeply the nesting runs.
///
/// `expect` is judged by scope *and* by whether it gives a reason, rather than
/// exempted outright. An item-scoped `#[expect(..., reason = "...")]` is the
/// sanctioned form and is left alone; a crate-scoped `#![expect(...)]` is not,
/// because one call anywhere in the crate fulfils it and the rest go
/// unreported. Measured: `#![expect(clippy::disallowed_methods)]` reports
/// neither the disallowed method nor an unfulfilled expectation, so nothing at
/// all is left to notice.
///
/// An item-scoped `expect` with no reason was exempted by an earlier draft,
/// which said the sanctioned form carried one and then never asked. The
/// reason is the whole of what distinguishes the sanctioned form from a
/// quieter `allow`, so it is now required and must not be blank.
pub(super) fn suppressed_by(meta: &Meta, inner: bool) -> Vec<String> {
    let Ok(list) = meta.require_list() else {
        return Vec::new();
    };
    match render_path(meta.path()).as_str() {
        "allow" => allowed_lints(list),
        "expect" if inner || !has_a_reason(list) => allowed_lints(list),
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
