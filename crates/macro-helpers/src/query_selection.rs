//! `query_selection!`: the select field enum and the selection a read carries.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::field_set::expand_with;
use crate::naming::variant;

/// The expansion behind `query_selection!`.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    expand_with(input, |set| {
        let entity_name = set.entity.to_string();
        let field = format_ident!("{entity_name}SelectField");
        let selection = format_ident!("{entity_name}Selection");

        let mut variants = Vec::new();
        let mut all = Vec::new();
        let mut names = Vec::new();
        for name in &set.fields {
            let variant = variant(&name.to_string());
            let wire = name.to_string();
            let doc = format!("The `{wire}` column.");
            variants.push(quote! {
                #[doc = #doc]
                #[serde(rename = #wire)]
                #variant,
            });
            all.push(quote! { Self::#variant, });
            names.push(quote! { Self::#variant => #wire, });
        }
        let field_doc = format!("A column of `{entity_name}` a read may select.");
        let selection_doc =
            format!("The columns a `{entity_name}` read selects; empty selects every one.");

        quote! {
            #[doc = #field_doc]
            #[derive(
                Clone,
                Copy,
                Debug,
                Eq,
                Hash,
                Ord,
                PartialEq,
                PartialOrd,
                serde::Deserialize,
                serde::Serialize,
            )]
            pub enum #field {
                #(#variants)*
            }

            impl #field {
                /// Every field, in declaration order.
                pub const ALL: &'static [Self] = &[#(#all)*];

                /// The column this field names.
                #[must_use]
                pub const fn as_str(self) -> &'static str {
                    match self {
                        #(#names)*
                    }
                }

                /// The select-list entry for this column.
                #[must_use]
                pub fn select(self) -> SelectColumn {
                    SelectColumn::Name(self.as_str().to_owned())
                }

                /// The field a wire column names; `None` when the entity has no such column.
                #[must_use]
                pub fn from_column(column: &str) -> Option<Self> {
                    Self::ALL.iter().copied().find(|field| field.as_str() == column)
                }
            }

            impl std::fmt::Display for #field {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(self.as_str())
                }
            }

            #[doc = #selection_doc]
            #[doc = ""]
            #[doc = "On the wire it is an object naming each column, as the TypeScript read sends it."]
            #[derive(Clone, Debug, Default, PartialEq)]
            pub struct #selection(pub Vec<#field>);

            impl serde::Serialize for #selection {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    use serde::ser::SerializeMap;

                    let mut map = serializer.serialize_map(Some(self.0.len()))?;
                    for field in &self.0 {
                        map.serialize_entry(field.as_str(), &true)?;
                    }
                    map.end()
                }
            }

            impl<'de> serde::Deserialize<'de> for #selection {
                fn deserialize<D: serde::Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Self, D::Error> {
                    flat_filter::selection(deserializer, #field::from_column).map(Self)
                }
            }

            impl FromIterator<#field> for #selection {
                fn from_iter<I: IntoIterator<Item = #field>>(fields: I) -> Self {
                    Self(fields.into_iter().collect())
                }
            }
        }
    })
}
