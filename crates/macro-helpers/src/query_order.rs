//! `query_order!`: the order field enum and the ordering clause a read carries.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::field_set::expand_with;
use crate::naming::variant;

/// The expansion behind `query_order!`.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    expand_with(input, |set| {
        let entity_name = set.entity.to_string();
        let field = format_ident!("{entity_name}OrderField");
        let order = format_ident!("{entity_name}Order");

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
        let field_doc =
            format!("A column of `{entity_name}` a read may order by, from `@queryOrderBy`.");
        let order_doc = format!(
            "One ordering clause of a `{entity_name}` read, built from `{entity_name}OrderField`."
        );

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

                /// Ascending order on this column.
                #[must_use]
                pub fn asc(self) -> #order {
                    #order {
                        field: self,
                        direction: SortDirection::Asc,
                    }
                }

                /// Descending order on this column.
                #[must_use]
                pub fn desc(self) -> #order {
                    #order {
                        field: self,
                        direction: SortDirection::Desc,
                    }
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

            #[doc = #order_doc]
            #[doc = ""]
            #[doc = "On the wire it is a `[\"column\", \"asc\" | \"desc\"]` tuple, as the TypeScript read sends it."]
            #[derive(Clone, Debug, PartialEq)]
            pub struct #order {
                field: #field,
                direction: SortDirection,
            }

            impl serde::Serialize for #order {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    use serde::ser::SerializeTuple;

                    let mut tuple = serializer.serialize_tuple(2)?;
                    tuple.serialize_element(self.field.as_str())?;
                    tuple.serialize_element(&self.direction)?;
                    tuple.end()
                }
            }

            impl<'de> serde::Deserialize<'de> for #order {
                fn deserialize<D: serde::Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Self, D::Error> {
                    let (field, direction) =
                        flat_filter::sort(deserializer, #field::from_column)?;
                    Ok(Self { field, direction })
                }
            }

            impl #order {
                /// The builder's ordering clause.
                #[must_use]
                pub fn into_sort(self) -> Sort {
                    Sort {
                        column: self.field.as_str().to_owned(),
                        direction: self.direction,
                        nulls: None,
                    }
                }
            }
        }
    })
}
