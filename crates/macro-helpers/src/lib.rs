//! The macros the generated Rust code invokes, one module per shape it repeats.
//!
//! A proc-macro entry point has to sit at the crate root, so each module here holds the expansion
//! and the root names it. A generated module invokes the macro and nothing else from here.
//! See docs/rust-queries.md.

mod field_set;
mod naming;
mod query_filters;
mod query_order;
mod query_selection;

use proc_macro::TokenStream;

/// Write one filter enum per column, its entity's filter enum, and every lowering between them.
///
/// ```ignore
/// query_filters! {
///     Widget {
///         id: WidgetId, Uuid, [eq],
///         createdAt: chrono::DateTime<chrono::Utc>, Timestamptz, [gte, lte, between],
///     }
/// }
/// ```
#[proc_macro]
pub fn query_filters(input: TokenStream) -> TokenStream {
    query_filters::expand(input)
}

/// Write the select field enum of an entity and the selection a read of it carries.
///
/// ```ignore
/// query_selection! {
///     Widget {
///         id,
///         tenantId,
///     }
/// }
/// ```
#[proc_macro]
pub fn query_selection(input: TokenStream) -> TokenStream {
    query_selection::expand(input.into()).into()
}

/// Write the order field enum of an entity and the ordering clause a read of it carries.
///
/// ```ignore
/// query_order! {
///     Widget {
///         createdAt,
///         updatedAt,
///     }
/// }
/// ```
#[proc_macro]
pub fn query_order(input: TokenStream) -> TokenStream {
    query_order::expand(input.into()).into()
}
