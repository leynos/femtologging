//! Reading a `macro_rules!` arm's pattern, to decide whether a forwarded
//! attribute could reach a policy call.
//!
//! An attribute whose path the caller supplies is only a suppression if the
//! arm can put it over an environment access. That question is about what the
//! arm's matcher declares and what its transcriber writes, not about the shape
//! of an attribute, so it is asked here rather than beside the token walk.
//!
//! Getting it wrong in either direction is costly. Too wide and the ordinary
//! doc-forwarding idiom becomes a finding, and a contract that reports a false
//! positive gets switched off. Too narrow and a forwarded attribute over
//! supplied items goes unreported.
//!
//! Splitting a `macro_rules!` body into its arms belongs here for the same
//! reason: an arm is a pattern and a transcriber read together, and the
//! pattern is only ever read to answer the question above.

use proc_macro2::{TokenStream, TokenTree};

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
pub(super) fn could_cover_a_policy_call(pattern: &TokenStream, transcriber: &TokenStream) -> bool {
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
pub(super) fn macro_arms(stream: TokenStream) -> Vec<(TokenStream, TokenStream)> {
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
