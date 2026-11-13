//! The macros the generated Rust code invokes, one module per shape it repeats.
//!
//! A proc-macro entry point has to sit at the crate root, so each module here holds the expansion
//! and the root names it. A generated module invokes the macro and nothing else from here.
//! See docs/rust-queries.md.

mod flat_query_filters;

use proc_macro::TokenStream;

/// Write one filter enum per column, with its wire form and the typed filter the planner binds.
///
/// ```ignore
/// flat_query_filters! {
///     Widget {
///         id: WidgetId, Uuid, [eq],
///         createdAt: chrono::DateTime<chrono::Utc>, Timestamptz, [gte, lte, between],
///     }
/// }
/// ```
#[proc_macro]
pub fn flat_query_filters(input: TokenStream) -> TokenStream {
    flat_query_filters::expand(input)
}
