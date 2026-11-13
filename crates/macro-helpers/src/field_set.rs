//! The `Entity { field, field }` syntax `query_selection!` and `query_order!` share.

use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

/// One entity and the columns a generated enum over it names.
pub(crate) struct FieldSet {
    /// The entity the fields belong to, e.g. `Widget`.
    pub(crate) entity: Ident,
    /// The columns, in declaration order, spelled as the wire spells them.
    pub(crate) fields: Vec<Ident>,
}

impl Parse for FieldSet {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let entity: Ident = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut fields = Vec::new();
        while !body.is_empty() {
            fields.push(body.parse()?);
            if body.peek(Token![,]) {
                body.parse::<Token![,]>()?;
            } else if !body.is_empty() {
                return Err(body.error("expected a `,` between fields"));
            }
        }
        Ok(Self { entity, fields })
    }
}

/// Parse one invocation and hand the field set to `expand`, turning a parse error into tokens.
pub(crate) fn expand_with(
    input: TokenStream,
    expand: impl FnOnce(&FieldSet) -> TokenStream,
) -> TokenStream {
    match syn::parse2::<FieldSet>(input) {
        Ok(set) => expand(&set),
        Err(error) => error.to_compile_error(),
    }
}
