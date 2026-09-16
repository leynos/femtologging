//! Walking token streams, where an attribute is invisible to `syn`'s visitor.
//!
//! A `macro_rules!` transcriber is an opaque token stream to `syn`, yet Clippy
//! expands it and honours whatever attribute it writes. Two further shapes are
//! not complete attributes where they are written at all, and are refused
//! structurally rather than by their meta: an attribute whose path the caller
//! supplies, and an `include!` of a file the scan cannot read.

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use syn::{AttrStyle, Attribute, LitStr, Macro, Meta, visit::Visit};

use super::SOURCE_EXTENSION;
use super::discovery::is_walkable;
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
    /// `rustc` resolves an `include!` against the file that writes it, so the
    /// inclusion rule cannot judge a target without knowing where it sits.
    pub(super) path: Utf8PathBuf,
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
                if let Some(finding) = foreign_inclusion(&mac.tokens, &self.path) {
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

/// Return whether `stream` writes an environment access at any depth.
///
/// The call the arm writes sits inside the item's block, which is one group
/// down, so the search recurses. Reading only the top level found nothing and
/// let the route through.
///
/// `env` alone is not an access. An earlier draft matched any identifier of
/// that name, so `fn build(env: &str)` entered the environment branch and a
/// forwarded attribute over it became a finding the arm had not earned. What
/// makes it an access is what follows: `env::var` and `std::env::var` read the
/// module, and `env!` and `option_env!` read the environment at compile time.
/// A parameter, a field or a local named `env` is followed by a single colon,
/// a comma or a closing delimiter, and none of those is a path or a macro.
fn mentions_env(stream: &TokenStream) -> bool {
    let tokens: Vec<TokenTree> = stream.clone().into_iter().collect();
    tokens.iter().enumerate().any(|(index, token)| match token {
        TokenTree::Ident(ident) => {
            (ident == "env" || ident == "option_env") && reads_the_module(&tokens, index)
        }
        TokenTree::Group(group) => mentions_env(&group.stream()),
        TokenTree::Punct(_) | TokenTree::Literal(_) => false,
    })
}

/// Return whether the identifier at `index` is used as a path or a macro.
///
/// `env::var` is a path, and `env!("HOME")` is a macro; both read the
/// environment. A `:` that is not part of a `::` introduces a type, which is
/// how a parameter or a field named `env` is written, and is not an access.
fn reads_the_module(tokens: &[TokenTree], index: usize) -> bool {
    match tokens.get(index + 1) {
        Some(TokenTree::Punct(punct)) if punct.as_char() == '!' => true,
        Some(TokenTree::Punct(punct)) if punct.as_char() == ':' => {
            matches!(tokens.get(index + 2), Some(TokenTree::Punct(next)) if next.as_char() == ':')
        }
        _ => false,
    }
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
    declares_code_fragment_anywhere(pattern)
}

/// Return whether a matcher declares a code-carrying fragment at any depth.
///
/// A repetition puts its specifier inside a group: `$($body:item)*` declares
/// an `item` that a top-level read never sees. Reading only the matcher's own
/// tokens therefore judged such an arm unreachable, and a forwarded attribute
/// over the items its caller supplies produced no finding at all.
fn declares_code_fragment_anywhere(pattern: &TokenStream) -> bool {
    let tokens: Vec<TokenTree> = pattern.clone().into_iter().collect();
    if (0..tokens.len()).any(|index| declares_code_fragment(&tokens, index)) {
        return true;
    }
    tokens.iter().any(|token| match token {
        TokenTree::Group(group) => declares_code_fragment_anywhere(&group.stream()),
        _ => false,
    })
}

/// Return whether a fragment specifier at `index` names something that can
/// carry code.
///
/// A specifier is written `$name:kind`, so the colon marks one and the
/// identifier after it is the kind. The kinds that can carry a call are in
/// [`CODE_FRAGMENTS`]; an `ident`, a `ty`, a `lifetime` or a `literal` cannot.
fn declares_code_fragment(tokens: &[TokenTree], index: usize) -> bool {
    if !matches!(tokens.get(index), Some(TokenTree::Punct(punct)) if punct.as_char() == ':') {
        return false;
    }
    matches!(
        tokens.get(index + 1),
        Some(TokenTree::Ident(ident)) if CODE_FRAGMENTS.contains(&ident.to_string().as_str())
    )
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
fn foreign_inclusion(tokens: &TokenStream, including: &Utf8Path) -> Option<String> {
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
