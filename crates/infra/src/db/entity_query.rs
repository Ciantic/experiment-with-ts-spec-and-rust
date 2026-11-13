//! The builder every generated entity query aliases, and the read it lowers to.

use std::fmt;

use serde::de::DeserializeOwned;

use super::{FilterKind, PlannedQuery, ReadError, ReadOptions, SqlExecutor, decode_rows};

/// The planner's read, lowered from an entity query.
///
/// Opaque on purpose: it holds the crate's own planner types, which no caller names.
#[doc(hidden)]
pub struct LoweredRead(pub(crate) ReadOptions);

impl LoweredRead {
    /// The statement this read forms against `table`.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Build`] when the read does not form a valid statement.
    pub(crate) fn plan(self, table: &str) -> Result<PlannedQuery, ReadError> {
        self.0.plan(table, None)
    }
}

/// What a query needs to know about its entity, so the builder itself is written once.
///
/// `scripts/infra/generate-queries-rs.ts` implements this for each entity's model type, which is
/// what the entity's `{Entity}Query` alias is an [`EntityQuery`] over.
pub trait ReadEntity {
    /// The row shape a query decodes into.
    type Select: DeserializeOwned;
    /// A column a query may select.
    type SelectField;
    /// The selected columns; empty selects every one.
    type Selection: Clone
        + fmt::Debug
        + Default
        + FromIterator<Self::SelectField>
        + PartialEq
        + serde::Serialize
        + DeserializeOwned;
    /// A filter over the entity's columns.
    type Filter: Clone + fmt::Debug + PartialEq + serde::Serialize + DeserializeOwned;
    /// One ordering clause.
    type Order: Clone + fmt::Debug + PartialEq + serde::Serialize + DeserializeOwned;

    /// The table a query of this entity plans against.
    const TABLE: &'static str;

    /// A group of `filters`, combined the way `kind` says.
    fn group(kind: FilterKind, filters: Vec<Self::Filter>) -> Self::Filter;

    /// Lower the query's parts to the read the planner takes.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Encode`] when a filter value has no JSON form.
    fn lower(
        select: Self::Selection,
        filters: Vec<Self::Filter>,
        order: Vec<Self::Order>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<LoweredRead, ReadError>;
}

/// What a query of an entity selects, filters, orders, and pages.
///
/// A generated `{Entity}Query` is this type over the entity's model, so a selection or an ordering
/// clause can only name that entity's columns.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct EntityQuery<E: ReadEntity> {
    /// The selected columns; empty (or omitted) selects every column.
    #[serde(default)]
    pub select: E::Selection,
    /// The filters, combined with `and`; the same column may appear more than once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<E::Filter>,
    /// The ordering clauses, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<E::Order>,
    /// The maximum number of rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// The number of leading rows to skip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

impl<E: ReadEntity> Clone for EntityQuery<E> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            filters: self.filters.clone(),
            order: self.order.clone(),
            limit: self.limit,
            offset: self.offset,
        }
    }
}

impl<E: ReadEntity> fmt::Debug for EntityQuery<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntityQuery")
            .field("select", &self.select)
            .field("filters", &self.filters)
            .field("order", &self.order)
            .field("limit", &self.limit)
            .field("offset", &self.offset)
            .finish()
    }
}

impl<E: ReadEntity> Default for EntityQuery<E> {
    fn default() -> Self {
        Self {
            select: E::Selection::default(),
            filters: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
        }
    }
}

impl<E: ReadEntity> PartialEq for EntityQuery<E> {
    fn eq(&self, other: &Self) -> bool {
        self.select == other.select
            && self.filters == other.filters
            && self.order == other.order
            && self.limit == other.limit
            && self.offset == other.offset
    }
}

impl<E: ReadEntity> EntityQuery<E> {
    /// A query that selects `fields`.
    #[must_use]
    pub fn with_select(fields: impl IntoIterator<Item = E::SelectField>) -> Self {
        Self {
            select: fields.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Add a filter; the query combines them with `and`. The same column may be filtered twice.
    #[must_use]
    pub fn filter(mut self, filter: impl Into<E::Filter>) -> Self {
        self.filters.push(filter.into());
        self
    }

    /// Add filters of which any may match; the query's own list stays `and`ed.
    #[must_use]
    pub fn filter_or(mut self, filters: impl IntoIterator<Item = impl Into<E::Filter>>) -> Self {
        self.filters.push(E::group(
            FilterKind::Or,
            filters.into_iter().map(Into::into).collect(),
        ));
        self
    }

    /// Add filters of which all must match; one branch may itself be a group.
    #[must_use]
    pub fn filter_and(mut self, filters: impl IntoIterator<Item = impl Into<E::Filter>>) -> Self {
        self.filters.push(E::group(
            FilterKind::And,
            filters.into_iter().map(Into::into).collect(),
        ));
        self
    }

    /// Add an ordering clause, built by the entity's `{Entity}OrderField` helpers.
    #[must_use]
    pub fn order(mut self, order: E::Order) -> Self {
        self.order.push(order);
        self
    }

    /// Keep at most `limit` rows.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Skip `offset` leading rows.
    #[must_use]
    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// The statement this query runs: its SQL and the typed parameters it binds.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError`] when the query does not form a valid statement.
    pub fn as_sql(&self) -> Result<PlannedQuery, ReadError> {
        self.lower()?.plan(E::TABLE)
    }

    /// Run this query, decoding every row into the entity's selected shape.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError`] when the statement cannot be built, run, or decoded.
    pub async fn query(self, db: &impl SqlExecutor) -> Result<Vec<E::Select>, ReadError> {
        let read = E::lower(
            self.select,
            self.filters,
            self.order,
            self.limit,
            self.offset,
        )?;
        let plan = read.plan(E::TABLE)?;
        let rows = db.query(&plan.sql, &plan.params).await?;
        decode_rows(rows)
    }

    /// This query as the read the planner takes, without running it.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError`] when a filter value has no JSON form.
    fn lower(&self) -> Result<LoweredRead, ReadError> {
        E::lower(
            self.select.clone(),
            self.filters.clone(),
            self.order.clone(),
            self.limit,
            self.offset,
        )
    }
}

#[cfg(test)]
mod tests {
    use datagrid_query::{FilterOperator, SelectColumn, Sort, SortDirection};

    use super::{EntityQuery, LoweredRead, ReadEntity};
    use crate::db::{FilterKind, ReadError, ReadOptions, SqlType, TypedFilter};

    /// The entity the fixture plans against; nothing here comes from the spec.
    struct Widget;

    #[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    enum WidgetSelectField {
        Id,
        Name,
    }

    impl WidgetSelectField {
        fn select(self) -> SelectColumn {
            let column = match self {
                Self::Id => "id",
                Self::Name => "name",
            };
            SelectColumn::Name(column.to_owned())
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
    struct WidgetSelection(Vec<WidgetSelectField>);

    impl FromIterator<WidgetSelectField> for WidgetSelection {
        fn from_iter<I: IntoIterator<Item = WidgetSelectField>>(fields: I) -> Self {
            Self(fields.into_iter().collect())
        }
    }

    #[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    struct WidgetOrder(String);

    impl WidgetOrder {
        fn into_sort(self) -> Sort {
            Sort {
                column: self.0,
                direction: SortDirection::Desc,
                nulls: None,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    #[serde(untagged)]
    enum WidgetFilter {
        Name(String),
        Group {
            kind: FilterKind,
            filters: Vec<Self>,
        },
    }

    impl ReadEntity for Widget {
        type Select = serde_json::Value;
        type SelectField = WidgetSelectField;
        type Selection = WidgetSelection;
        type Filter = WidgetFilter;
        type Order = WidgetOrder;

        const TABLE: &'static str = "widget";

        fn group(kind: FilterKind, filters: Vec<Self::Filter>) -> Self::Filter {
            WidgetFilter::Group { kind, filters }
        }

        fn lower(
            select: Self::Selection,
            filters: Vec<Self::Filter>,
            order: Vec<Self::Order>,
            limit: Option<u64>,
            offset: Option<u64>,
        ) -> Result<LoweredRead, ReadError> {
            Ok(LoweredRead(ReadOptions {
                columns: select
                    .0
                    .into_iter()
                    .map(WidgetSelectField::select)
                    .collect(),
                filters: filters.into_iter().flat_map(typed_filters).collect(),
                sort: order.into_iter().map(WidgetOrder::into_sort).collect(),
                limit,
                offset,
            }))
        }
    }

    fn typed_filters(filter: WidgetFilter) -> Vec<TypedFilter> {
        match filter {
            WidgetFilter::Name(name) => vec![
                TypedFilter::one("name", SqlType::Text, FilterOperator::Eq, name)
                    .expect("a name encodes"),
            ],
            WidgetFilter::Group { kind, filters } => {
                let mut typed = Vec::new();
                for filter in filters {
                    typed.extend(typed_filters(filter));
                }
                match typed.len() {
                    // A one-branch group adds no parentheses, so it lowers to its branch.
                    1 => typed,
                    _ => vec![TypedFilter::group(kind.combinator(), typed)],
                }
            }
        }
    }

    fn widget() -> EntityQuery<Widget> {
        EntityQuery::default()
    }

    #[test]
    fn keeps_the_queries_own_filters_anded_and_a_group_ored() {
        let plan = widget()
            .filter(WidgetFilter::Name("a".to_owned()))
            .filter_or([
                WidgetFilter::Name("b".to_owned()),
                WidgetFilter::Name("c".to_owned()),
            ])
            .as_sql()
            .expect("the query plans");

        assert_eq!(
            plan.sql,
            r#"SELECT * FROM "widget" WHERE "name" = $1 AND ("name" = $2 OR "name" = $3)"#
        );
        assert_eq!(plan.params.len(), 3);
    }

    #[test]
    fn nests_a_group_the_list_does_not_hold() {
        let plan = widget()
            .filter_or([WidgetFilter::Name("a".to_owned())])
            .filter_and([WidgetFilter::Name("b".to_owned())])
            .as_sql()
            .expect("the query plans");

        assert_eq!(
            plan.sql,
            r#"SELECT * FROM "widget" WHERE "name" = $1 AND "name" = $2"#
        );
    }

    #[test]
    fn selects_the_fields_it_is_given_and_pages_the_result() {
        let plan = EntityQuery::<Widget>::with_select([WidgetSelectField::Name])
            .order(WidgetOrder("name".to_owned()))
            .limit(2)
            .offset(5)
            .as_sql()
            .expect("the query plans");

        assert_eq!(
            plan.sql,
            r#"SELECT "name" FROM "widget" ORDER BY "name" DESC LIMIT 2 OFFSET 5"#
        );
    }

    #[test]
    fn writes_a_selection_even_when_empty_and_omits_the_other_parts() {
        let json = serde_json::to_value(widget()).expect("a query serializes");

        assert_eq!(json, serde_json::json!({ "select": [] }));
    }

    #[test]
    fn reads_back_what_it_writes() {
        let query = EntityQuery::<Widget>::with_select([WidgetSelectField::Id])
            .filter(WidgetFilter::Name("a".to_owned()))
            .order(WidgetOrder("name".to_owned()))
            .limit(2);
        let json = serde_json::to_string(&query).expect("a query serializes");

        assert_eq!(
            serde_json::from_str::<EntityQuery<Widget>>(&json).expect("a query deserializes"),
            query
        );
    }

    #[test]
    fn rejects_a_key_it_does_not_know() {
        let error = serde_json::from_str::<EntityQuery<Widget>>(r#"{"select":[],"unknown":true}"#)
            .expect_err("an unknown key is rejected");

        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}
