//! The expansion behind `query_filters!`: one column's enum, wire form, and lowering.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::naming::{pascal_name, variant};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, Token, Type, braced, bracketed};

/// One operator a filter column may offer, in the query builder's spelling.
struct Operator {
    /// The wire spelling, as `@queryFilter ops=` names it and the entry's key ends with.
    spelling: &'static str,
    /// The planner's spelling, as `FilterOperator` names it.
    planner: &'static str,
    /// The values the comparison binds.
    arity: Arity,
}

impl Operator {
    const fn new(spelling: &'static str, planner: &'static str, arity: Arity) -> Self {
        Self {
            spelling,
            planner,
            arity,
        }
    }
}

/// Every operator a `@queryFilter ops=` may name.
const OPERATORS: &[Operator] = &[
    Operator::new("eq", "Eq", Arity::One),
    Operator::new("neq", "Neq", Arity::One),
    Operator::new("gt", "Gt", Arity::One),
    Operator::new("gte", "Gte", Arity::One),
    Operator::new("lt", "Lt", Arity::One),
    Operator::new("lte", "Lte", Arity::One),
    Operator::new("like", "Like", Arity::One),
    Operator::new("ilike", "ILike", Arity::One),
    Operator::new("notLike", "NotLike", Arity::One),
    Operator::new("notIlike", "NotILike", Arity::One),
    Operator::new("in", "In", Arity::List),
    Operator::new("notIn", "NotIn", Arity::List),
    Operator::new("any", "Any", Arity::List),
    Operator::new("notAny", "NotAny", Arity::List),
    Operator::new("isNull", "IsNull", Arity::None),
    Operator::new("isNotNull", "IsNotNull", Arity::None),
    Operator::new("between", "Between", Arity::Pair),
    Operator::new("notBetween", "NotBetween", Arity::Pair),
];

/// How many values one comparison binds.
#[derive(Clone, Copy, PartialEq)]
enum Arity {
    None,
    One,
    List,
    Pair,
}

/// One filterable column: its name, the model type a value takes, its SQL type, and its operators.
struct Column {
    name: Ident,
    ty: Type,
    sql: Ident,
    ops: Vec<Ident>,
}

/// The columns of one entity, keyed by the entity they filter.
struct FlatFilters {
    entity: Ident,
    columns: Vec<Column>,
}

impl Parse for FlatFilters {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let entity: Ident = input.parse()?;
        let body;
        braced!(body in input);
        let mut columns = Vec::new();
        while !body.is_empty() {
            let name: Ident = body.parse()?;
            body.parse::<Token![:]>()?;
            let ty: Type = body.parse()?;
            body.parse::<Token![,]>()?;
            let sql: Ident = body.parse()?;
            body.parse::<Token![,]>()?;
            let list;
            bracketed!(list in body);
            let ops: Punctuated<Ident, Token![,]> =
                list.parse_terminated(Ident::parse, Token![,])?;
            columns.push(Column {
                name,
                ty,
                sql,
                ops: ops.into_iter().collect(),
            });
            if body.peek(Token![,]) {
                body.parse::<Token![,]>()?;
            }
        }
        Ok(Self { entity, columns })
    }
}

/// Expand one `query_filters!` invocation: a filter enum per column, with its wire form and
/// the typed filter the planner binds.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let spec = match syn::parse::<FlatFilters>(input) {
        Ok(spec) => spec,
        Err(error) => return error.to_compile_error().into(),
    };
    let mut out = TokenStream2::new();
    for column in &spec.columns {
        match expand_column(&spec.entity, column) {
            Ok(tokens) => out.extend(tokens),
            Err(error) => return error.to_compile_error().into(),
        }
    }
    out.extend(expand_entity(&spec.entity, &spec.columns));
    out.into()
}

/// The entity's filter: one untagged variant per column, plus the group a read nests.
///
/// A read's own filter list is combined with `and`, so a column may appear more than once.
fn expand_entity(entity: &Ident, columns: &[Column]) -> TokenStream2 {
    let entity_name = entity.to_string();
    let filter = format_ident!("{entity_name}Filter");
    let mut variants = Vec::new();
    let mut into_impls = Vec::new();
    let mut lowering_arms = Vec::new();
    for column in columns {
        let variant = variant(&column.name.to_string());
        let column_filter = format_ident!(
            "{}{}Filter",
            entity_name,
            pascal_name(&column.name.to_string())
        );
        let doc = format!("A comparison on `{}`.", column.name);
        variants.push(quote! {
            #[doc = #doc]
            #variant(#column_filter),
        });
        into_impls.push(quote! {
            impl From<#column_filter> for #filter {
                fn from(filter: #column_filter) -> Self {
                    Self::#variant(filter)
                }
            }
        });
        lowering_arms.push(quote! {
            Self::#variant(filter) => Ok(vec![filter.into_typed_filter()?]),
        });
    }
    let doc = format!(
        "A filter over `{entity_name}`: a comparison on one of its columns, or a group of filters."
    );
    quote! {
        #[doc = #doc]
        #[doc = "A read's filters are combined with `and`, so a column may appear more than once."]
        #[doc = "See docs/rust-queries.md."]
        #[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
        #[serde(untagged)]
        pub enum #filter {
            #(#variants)*
            /// A group of filters, combined the way `kind` says.
            Group {
                /// Whether every branch must match or any may.
                kind: FilterKind,
                /// The branches; each is a comparison or, in turn, a group.
                filters: Vec<Self>,
            },
        }

        #(#into_impls)*

        impl #filter {
            /// The typed filters this names, in the order the builder binds them.
            ///
            /// # Errors
            ///
            /// Returns [`ReadError::Encode`] when a value has no JSON form.
            pub(crate) fn into_typed_filters(self) -> Result<Vec<TypedFilter>, ReadError> {
                match self {
                    #(#lowering_arms)*
                    Self::Group { kind, filters } => {
                        let mut typed = Vec::new();
                        for filter in filters {
                            typed.extend(filter.into_typed_filters()?);
                        }
                        Ok(match typed.len() {
                            0 => Vec::new(),
                            1 => typed,
                            _ => vec![TypedFilter::group(kind.combinator(), typed)],
                        })
                    }
                }
            }
        }
    }
}

/// One column's enum, its two serde impls, and its lowering.
fn expand_column(entity: &Ident, column: &Column) -> syn::Result<TokenStream2> {
    let entity_name = entity.to_string();
    let column_name = column.name.to_string();
    let name = format_ident!("{}{}Filter", entity_name, pascal_name(&column_name));
    let ty = &column.ty;
    let sql = &column.sql;

    let mut variants = Vec::new();
    let mut serialize_arms = Vec::new();
    let mut deserialize_arms = Vec::new();
    let mut lower_arms = Vec::new();
    let mut spellings = Vec::new();
    for op in &column.ops {
        let operator = operator(op)?;
        let variant = format_ident!("{}", pascal_name(operator.spelling));
        let planner = format_ident!("{}", operator.planner);
        let key = format!("{column_name}__{}", operator.spelling);
        spellings.push(operator.spelling.to_owned());
        match operator.arity {
            Arity::One => {
                variants.push(quote! { #variant(#ty) });
                serialize_arms.push(quote! {
                    Self::#variant(value) => flat_filter::serialize(#key, value, serializer),
                });
                deserialize_arms.push(quote! {
                    #key => Ok(Self::#variant(flat_filter::payload(key, value)?)),
                });
                lower_arms.push(quote! {
                    Self::#variant(value) => TypedFilter::one(
                        #column_name,
                        SqlType::#sql,
                        FilterOperator::#planner,
                        value,
                    )?,
                });
            }
            Arity::List => {
                variants.push(quote! { #variant(Vec<#ty>) });
                serialize_arms.push(quote! {
                    Self::#variant(values) => flat_filter::serialize(#key, values, serializer),
                });
                deserialize_arms.push(quote! {
                    #key => Ok(Self::#variant(flat_filter::payload(key, value)?)),
                });
                lower_arms.push(quote! {
                    Self::#variant(values) => TypedFilter::list(
                        #column_name,
                        SqlType::#sql,
                        FilterOperator::#planner,
                        values,
                    )?,
                });
            }
            Arity::Pair => {
                variants.push(quote! { #variant(#ty, #ty) });
                serialize_arms.push(quote! {
                    Self::#variant(lower, upper) => {
                        flat_filter::serialize(#key, &[lower, upper], serializer)
                    }
                });
                deserialize_arms.push(quote! {
                    #key => {
                        let (lower, upper) = flat_filter::payload(key, value)?;
                        Ok(Self::#variant(lower, upper))
                    }
                });
                lower_arms.push(quote! {
                    Self::#variant(lower, upper) => TypedFilter::pair(
                        #column_name,
                        SqlType::#sql,
                        FilterOperator::#planner,
                        lower,
                        upper,
                    )?,
                });
            }
            Arity::None => {
                variants.push(quote! { #variant });
                serialize_arms.push(quote! {
                    Self::#variant => flat_filter::serialize(#key, &true, serializer),
                });
                deserialize_arms.push(quote! {
                    #key => {
                        flat_filter::null_test(key, value)?;
                        Ok(Self::#variant)
                    }
                });
                lower_arms.push(quote! {
                    Self::#variant => TypedFilter::null_test(
                        #column_name,
                        SqlType::#sql,
                        FilterOperator::#planner,
                    ),
                });
            }
        }
    }
    let Some(first) = column.ops.first() else {
        return Err(syn::Error::new(
            column.name.span(),
            "a filter column needs at least one operator",
        ));
    };
    let first_key = format!("{column_name}__{}", operator(first)?.spelling);
    let doc = format!(
        "A comparison on `{entity_name}.{column_name}`, limited to the operators \
         `@queryFilter ops={}` whitelists.",
        spellings.join(",")
    );
    let wire_doc = format!("On the wire it is one flat entry, such as `{{\"{first_key}\": …}}`.");
    let unknown = format!("unknown filter `{{other}}` on `{entity_name}.{column_name}`");

    Ok(quote! {
        #[doc = #doc]
        #[doc = ""]
        #[doc = #wire_doc]
        #[derive(Clone, Debug, PartialEq)]
        pub enum #name {
            #(#variants,)*
        }

        impl serde::Serialize for #name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    #(#serialize_arms)*
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for #name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                flat_filter::deserialize(deserializer, |key, value| match key {
                    #(#deserialize_arms)*
                    other => Err(format!(#unknown)),
                })
            }
        }

        impl #name {
            #[doc = "The typed filter the planner binds."]
            #[doc = ""]
            #[doc = "# Errors"]
            #[doc = ""]
            #[doc = "Returns [`ReadError::Encode`] when the value has no JSON form."]
            pub(crate) fn into_typed_filter(self) -> Result<TypedFilter, ReadError> {
                Ok(match self {
                    #(#lower_arms)*
                })
            }
        }
    })
}

/// One operator's spelling and the values it takes.
fn operator(op: &Ident) -> syn::Result<&'static Operator> {
    let name = op.to_string();
    OPERATORS
        .iter()
        .find(|operator| operator.spelling == name)
        .ok_or_else(|| {
            syn::Error::new(
                op.span(),
                format!("`{name}` is not an operator a filter column may offer"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::pascal_name;

    #[test]
    fn names_a_column_as_its_filter_type_does() {
        assert_eq!(pascal_name("issueDate"), "IssueDate");
        assert_eq!(pascal_name("ilike"), "Ilike");
        assert_eq!(pascal_name("notIlike"), "NotIlike");
        assert_eq!(
            pascal_name("tenant_shared_permission"),
            "TenantSharedPermission"
        );
    }
}
