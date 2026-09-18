//! Walking token streams, where an attribute is invisible to `syn`'s visitor.
//!
//! A `macro_rules!` transcriber is an opaque token stream to `syn`, yet Clippy
//! expands it and honours whatever attribute it writes. One further shape is
//! not a complete attribute where it is written at all, and is refused
//! structurally rather than by its meta: an attribute whose path the caller
//! supplies.
//!
//! Two questions this module asks are answered elsewhere, because neither is
//! about the shape of tokens. `inclusion` judges an `include!` target by the
//! walk's rule for which paths are reachable. `matcher` reads an arm's pattern
//! for whether a forwarded attribute could ever reach a policy call.

use camino::Utf8PathBuf;
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use syn::{AttrStyle, Attribute, Macro, Meta, visit::Visit};

use super::inclusion::{foreign_inclusion, foreign_module_path};
use super::matcher::{could_cover_a_policy_call, macro_arms};
use super::meta::{render_attribute, render_path};

/// Collect every attribute in a parsed file, wherever it sits.
///
/// A visitor is used rather than a hand-rolled walk so attributes on nested
/// items, on function-local items and on expressions are all reached.
///
/// Macro bodies are walked too, as token streams. `syn` keeps the body of a
/// `macro_rules!` arm opaque, so an attribute written there never reaches
/// `visit_attribute`, and Clippy expands and honours it. Measured on this
/// crate's `clippy.toml`: a macro arm emitting
/// `#[allow(clippy::disallowed_methods)]` around a function that calls
/// `std::env::var` reports zero diagnostics, where the same file without the
/// attribute reports one.
#[derive(Default)]
pub(super) struct AttributeCollector {
    /// Each suppression found: the text to report, the meta to judge, and
    /// whether it was written at inner scope.
    pub(super) attributes: Vec<(String, Meta, bool)>,
    /// Each finding no meta describes, already worded as a report line.
    pub(super) structural: Vec<String>,
    /// The file being visited, relative to the crate directory.
    ///
    /// `rustc` resolves an `include!` and a `#[path]` against the file that
    /// writes it, so neither rule can judge a target without knowing where it
    /// sits.
    pub(super) path: Utf8PathBuf,
}

impl<'ast> Visit<'ast> for AttributeCollector {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        // `#[path]` names a second source the same way `include!` does, and it
        // is an ordinary attribute rather than a macro, so it is judged here
        // rather than in `visit_macro`.
        if let Some(finding) = foreign_module_path(&attribute.meta, &self.path) {
            self.structural.push(finding);
        }
        self.attributes.push((
            render_attribute(attribute),
            attribute.meta.clone(),
            matches!(attribute.style, AttrStyle::Inner(_)),
        ));
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        match render_path(&mac.path).rsplit("::").next() {
            Some("macro_rules") => {
                for (pattern, transcriber) in macro_arms(mac.tokens.clone()) {
                    let reachable = could_cover_a_policy_call(&pattern, &transcriber);
                    collect_from_tokens(transcriber, reachable, self);
                }
            }
            Some("include") => {
                if let Some(finding) = foreign_inclusion(&mac.tokens, &self.path) {
                    self.structural.push(finding);
                }
            }
            _ => {}
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Collect attribute-shaped token sequences from a `macro_rules!` transcriber.
///
/// An attribute is `#`, optionally `!`, then a bracketed group. Recursing
/// through every group reaches an attribute at any depth, including one inside
/// a nested macro. A token walk cannot mistake prose for policy the way a text
/// scan can: a string literal is one token, never a `#` followed by brackets.
fn collect_from_tokens(stream: TokenStream, reachable: bool, collector: &mut AttributeCollector) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token
            && !is_invocation_argument(&tokens, index)
        {
            collect_from_tokens(group.stream(), reachable, collector);
        }
        record_attribute(&tokens, index, reachable, collector);
    }
}

/// Return whether the group at `index` is an ordinary macro's argument list.
///
/// A macro decides what its arguments become, and some of them become nothing
/// that carries policy: `stringify!(#[allow(clippy::all)])` emits a string, and
/// `discards!(#[allow(...)] fn f() {})` emits whatever its own arms say. The
/// tokens between the brackets are an argument, not an attribute, so reading
/// them as one is a false positive the arm never wrote, and a contract that
/// reports a false positive gets switched off.
///
/// An invocation is `ident !` then a delimited group.
///
/// A `macro_rules!` definition needs no exception. Its name sits between the
/// bang and the body, so the body is preceded by that name rather than by `!`
/// and is never read as an argument group. An exception for it was written
/// first and removed: no fixture could fail its absence, and a branch nothing
/// can fail is a comment that looks like a rule.
fn is_invocation_argument(tokens: &[TokenTree], index: usize) -> bool {
    let Some(TokenTree::Punct(bang)) = index.checked_sub(1).and_then(|at| tokens.get(at)) else {
        return false;
    };
    if bang.as_char() != '!' {
        return false;
    }
    matches!(
        index.checked_sub(2).and_then(|at| tokens.get(at)),
        Some(TokenTree::Ident(_))
    )
}

/// Return the scope marker and bracketed group of the attribute at `index`.
///
/// An attribute is `#`, optionally `!`, then a bracketed group. Reading that
/// shape is a question of its own, separate from what is then done with the
/// group, and naming it keeps the walk above about walking.
fn attribute_shape_at(tokens: &[TokenTree], index: usize) -> Option<(&'static str, &Group)> {
    if !matches!(tokens.get(index), Some(TokenTree::Punct(punct)) if punct.as_char() == '#') {
        return None;
    }
    let mut next = index + 1;
    let mut bang = "";
    if matches!(tokens.get(next), Some(TokenTree::Punct(punct)) if punct.as_char() == '!') {
        bang = "!";
        next += 1;
    }
    let TokenTree::Group(group) = tokens.get(next)? else {
        return None;
    };
    (group.delimiter() == Delimiter::Bracket).then_some((bang, group))
}

/// Record the attribute at `index`, as a structural finding or as a meta.
///
/// A forwarded path is refused where it is written, because it is not an
/// attribute a `Meta` can describe. Anything else that parses is kept for the
/// same judgement a parsed attribute gets, rather than a second, weaker test
/// written for tokens.
fn record_attribute(
    tokens: &[TokenTree],
    index: usize,
    reachable: bool,
    collector: &mut AttributeCollector,
) {
    let Some((bang, group)) = attribute_shape_at(tokens, index) else {
        return;
    };
    if let Some(finding) = forwarded_path(&group.stream(), bang, reachable) {
        collector.structural.push(finding);
    } else if let Ok(meta) = syn::parse2::<Meta>(group.stream()) {
        collector.attributes.push((
            format!("#{bang}[{}]", group.stream()),
            meta,
            !bang.is_empty(),
        ));
    }
}

/// Return a finding if an attribute in a transcriber forwards its own path.
///
/// `#[$attr]` is written by the arm and completed by the caller, so the arm
/// cannot be read for what it applies and the invocation carries no `#` for a
/// walk to notice. Neither half is a suppression on its own, and together they
/// are: invoked as `forward!(allow(clippy::disallowed_methods), ...)`, the
/// expansion silences every call the item contains.
///
/// Two things keep the rule narrow. Only a forwarded *path* is refused: an
/// attribute whose path is written out cannot become `allow`, however much of
/// its argument is forwarded, so `#[doc = $text]` and `#[derive($traits)]` are
/// judged as any other attribute and report nothing. And an outer forwarded
/// path is refused only where the arm could put it over a policy call, which
/// leaves the `$(#[$meta:meta])*` doc-forwarding idiom alone.
///
/// An inner forwarded path is refused wherever it appears. `#![$attr]` applies
/// to everything enclosing it rather than to one item, so there is no call it
/// could fail to cover, and nothing to weigh.
fn forwarded_path(stream: &TokenStream, bang: &str, reachable: bool) -> Option<String> {
    let mut tokens = stream.clone().into_iter();
    let first = tokens.next()?;
    if !matches!(&first, TokenTree::Punct(punct) if punct.as_char() == '$') {
        return None;
    }
    let inner = !bang.is_empty();
    if !inner && !reachable {
        return None;
    }
    Some(format!(
        "#{bang}[{stream}] forwards its own path, which the caller can complete \
         with `allow`; write the attribute out, or take the item rather than \
         the attribute"
    ))
}
