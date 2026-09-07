//! Procedural attributes for scoping allowances around generated test code.

use proc_macro::TokenStream;
use quote::quote;

/// Scope the `unused_braces` allowance emitted by a single-line `rstest` fixture.
///
/// The allowance is attached to the supplied item in the generated token
/// stream, so fixture source remains free of lint suppressions.
#[proc_macro_attribute]
pub fn allow_fixture_expansion_lints(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let fixture_item: proc_macro2::TokenStream = item.into();
    quote! {
        #[allow(unused_braces, reason = "rstest emits braces for single-line fixtures")]
        #fixture_item
    }
    .into()
}
