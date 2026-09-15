//! Walking token streams, where an attribute is invisible to `syn`'s visitor.
//!
//! A `macro_rules!` transcriber is an opaque token stream to `syn`, yet Clippy
//! expands it and honours whatever attribute it writes. Two further shapes are
//! not complete attributes where they are written at all, and are refused
//! structurally rather than by their meta: an attribute whose path the caller
//! supplies, and an `include!` of a file the scan cannot read.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::{AttrStyle, Attribute, Macro, Meta, visit::Visit};

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
}

impl<'ast> Visit<'ast> for AttributeCollector {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
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
                if let Some(finding) = foreign_inclusion(&mac.tokens) {
                    self.structural.push(finding);
                }
            }
            _ => {}
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Return each arm of a `macro_rules!` body as its pattern and transcriber.
///
/// Only a transcriber is expanded, so only a transcriber can suppress
/// anything. The arms' patterns are not output, and the arguments of an
/// ordinary macro invocation may be discarded by the macro it is handed to:
/// walking either reports an attribute that never reaches the compiler, and a
/// contract that reports a false positive gets switched off.
///
/// The pattern comes back with it because the fragment specifiers declared
/// there decide whether a forwarded attribute in the transcriber could bear on
/// the policy.
///
/// An arm is `(pattern) => {transcriber};`, so each group following a `=>` is
/// a transcriber and the group before the `=>` is its pattern.
fn macro_arms(stream: TokenStream) -> Vec<(TokenStream, TokenStream)> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, TokenTree::Punct(punct) if punct.as_char() == '=') {
            continue;
        }
        if !matches!(tokens.get(index + 1), Some(TokenTree::Punct(punct)) if punct.as_char() == '>')
        {
            continue;
        }
        let Some(TokenTree::Group(transcriber)) = tokens.get(index + 2) else {
            continue;
        };
        let pattern = match index.checked_sub(1).and_then(|before| tokens.get(before)) {
            Some(TokenTree::Group(group)) => group.stream(),
            _ => TokenStream::new(),
        };
        found.push((pattern, transcriber.stream()));
    }
    found
}

/// Fragment specifiers whose value can carry an environment access.
///
/// A caller supplying one of these supplies code, so an attribute forwarded
/// over it can cover a call the arm never mentions. An `ident`, a `ty`, a
/// `lifetime` or a `literal` cannot carry a call, which is what keeps the
/// doc-forwarding idiom below out of the findings.
const CODE_FRAGMENTS: [&str; 5] = ["item", "block", "stmt", "expr", "tt"];

/// Return whether `stream` names the `env` module at any depth.
///
/// The call the arm writes sits inside the item's block, which is one group
/// down, so the search recurses. Reading only the top level found nothing and
/// let the route through.
fn mentions_env(stream: &TokenStream) -> bool {
    stream.clone().into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == "env",
        TokenTree::Group(group) => mentions_env(&group.stream()),
        TokenTree::Punct(_) | TokenTree::Literal(_) => false,
    })
}

/// Return whether an arm could put a forwarded attribute over a policy call.
///
/// Either the arm writes the access itself, in which case `env` appears among
/// its tokens, or it forwards a fragment that the caller fills with code.
/// `$(#[$meta:meta])* $name:ident, $field:ident, $ty:ty` does neither: it is
/// the ordinary way to carry doc comments onto a generated setter, it appears
/// twice in this crate's own builders, and reporting it would be the false
/// positive that gets a contract switched off.
fn could_cover_a_policy_call(pattern: &TokenStream, transcriber: &TokenStream) -> bool {
    if mentions_env(transcriber) {
        return true;
    }
    let tokens: Vec<TokenTree> = pattern.clone().into_iter().collect();
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token, TokenTree::Punct(punct) if punct.as_char() == ':')
            && matches!(
                tokens.get(index + 1),
                Some(TokenTree::Ident(ident))
                    if CODE_FRAGMENTS.contains(&ident.to_string().as_str())
            )
    })
}

/// Return a finding if an `include!` names a target that is not Rust source.
///
/// `rustc` parses an included file as Rust whatever its extension, so
/// `include!("fixture.rs.txt")` compiles the fixture's contents into this
/// crate. An `allow` written there suppresses the policy for the calls around
/// it, and an enclosing `expect` stays fulfilled, so nothing warns. The scan
/// cannot read the target, because the target need not exist when the scan
/// runs, so the inclusion itself is the finding.
///
/// A literal `.rs` path is not a finding: such a file is scanned in its own
/// right, being a `.rs` file under a governed root. `include_str!` and
/// `include_bytes!` are not source inclusion at all and never reach here.
fn foreign_inclusion(tokens: &TokenStream) -> Option<String> {
    let rendered = tokens.to_string();
    let target = tokens.clone().into_iter().next().and_then(|token| {
        let TokenTree::Literal(literal) = token else {
            return None;
        };
        let text = literal.to_string();
        let trimmed = text.strip_prefix('"')?.strip_suffix('"')?;
        Some(trimmed.to_owned())
    });
    match target {
        Some(path) if path.ends_with(".rs") => None,
        Some(path) => Some(format!(
            "include!(\"{path}\") compiles a file the scan cannot see as Rust; \
             name a `.rs` path, which is scanned in its own right"
        )),
        None => Some(format!(
            "include!({rendered}) names a target the scan cannot resolve; \
             name a literal `.rs` path, which is scanned in its own right"
        )),
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
        if let TokenTree::Group(group) = token {
            collect_from_tokens(group.stream(), reachable, collector);
        }
        if !matches!(token, TokenTree::Punct(punct) if punct.as_char() == '#') {
            continue;
        }
        let mut next = index + 1;
        let mut bang = "";
        if matches!(tokens.get(next), Some(TokenTree::Punct(punct)) if punct.as_char() == '!') {
            bang = "!";
            next += 1;
        }
        let Some(TokenTree::Group(group)) = tokens.get(next) else {
            continue;
        };
        if group.delimiter() != Delimiter::Bracket {
            continue;
        }
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
