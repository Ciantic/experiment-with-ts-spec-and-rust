use datagrid_query::{
    Combinator, Filter, FilterCondition, FilterGroup, FilterOperator, QueryOptions, QuerySource,
    SelectColumn, Sort, Value,
};

use super::comparison::Comparison;
use super::sql_type::SqlType;

/// One bind parameter: the value the caller supplied, and the column type it is compared with.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub sql_type: SqlType,
    pub value: Value,
}

impl Param {
    #[must_use]
    pub const fn new(sql_type: SqlType, value: Value) -> Self {
        Self { sql_type, value }
    }
}

/// A generated statement: its SQL and the typed parameters it binds.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedQuery {
    pub sql: String,
    pub params: Vec<Param>,
}

/// A filter condition that carries the Postgres type of the column it targets.
///
/// The query builder renders the condition; the type is what a driver needs to encode the
/// parameter, because the builder's [`Value`] does not name a SQL type on its own.
#[derive(Clone, Debug, PartialEq)]
pub enum TypedFilter {
    /// One comparison on one typed column; an `any` filter binds an array of it.
    Condition {
        sql_type: SqlType,
        condition: FilterCondition,
    },
    /// A nested `and` / `or` group; each child keeps its own column type.
    Group {
        combinator: Combinator,
        filters: Vec<Self>,
    },
}

impl TypedFilter {
    /// A condition on a typed column, with `value` bound as one parameter.
    #[must_use]
    pub(crate) fn condition(
        column: &str,
        sql_type: SqlType,
        operator: FilterOperator,
        value: Option<Value>,
    ) -> Self {
        Self::Condition {
            sql_type,
            condition: FilterCondition {
                column: column.to_owned(),
                operator,
                value,
            },
        }
    }

    /// Turn a column's typed comparison into the builder's condition.
    ///
    /// # Errors
    ///
    /// Returns [`super::ReadError::Encode`] when a value has no JSON form.
    pub(crate) fn comparison<T: super::BindValue>(
        column: &str,
        sql_type: SqlType,
        comparison: Comparison<T>,
    ) -> Result<Self, super::ReadError> {
        let (operator, value) = match comparison {
            Comparison::Eq(value) => (FilterOperator::Eq, Some(value.to_bind_value()?)),
            Comparison::Ne(value) => (FilterOperator::Neq, Some(value.to_bind_value()?)),
            Comparison::Any(values) => {
                let mut bound = Vec::with_capacity(values.len());
                for value in &values {
                    bound.push(value.to_bind_value()?);
                }
                (FilterOperator::Any, Some(Value::Array(bound)))
            }
            Comparison::IsNull => (FilterOperator::IsNull, None),
            Comparison::IsNotNull => (FilterOperator::IsNotNull, None),
        };
        Ok(Self::condition(column, sql_type, operator, value))
    }

    /// A nested `and` / `or` group.
    #[must_use]
    pub(crate) fn group(combinator: Combinator, filters: Vec<Self>) -> Self {
        Self::Group {
            combinator,
            filters,
        }
    }

    /// The builder's filter for this condition or group.
    #[must_use]
    pub(crate) fn into_filter(self) -> Filter {
        match self {
            Self::Condition { condition, .. } => Filter::Condition(condition),
            Self::Group {
                combinator,
                filters,
            } => Filter::Group(FilterGroup {
                combinator,
                filters: filters.into_iter().map(Self::into_filter).collect(),
            }),
        }
    }

    /// The types of the parameters this filter binds, in the order the builder numbers them.
    #[must_use]
    pub(crate) fn param_types(&self) -> Vec<SqlType> {
        let mut types = Vec::new();
        self.collect_param_types(&mut types);
        types
    }

    /// Mirror the builder's parameter order: one value per comparison, one per `in` element,
    /// one array for `any`, two for `between`, and none for a null test.
    fn collect_param_types(&self, types: &mut Vec<SqlType>) {
        match self {
            Self::Condition {
                sql_type,
                condition,
            } => match condition.operator {
                // A null or missing value renders `IS [NOT] NULL`, which binds nothing.
                FilterOperator::Eq | FilterOperator::Neq if is_absent(&condition.value) => {}
                FilterOperator::Eq
                | FilterOperator::Neq
                | FilterOperator::Gt
                | FilterOperator::Gte
                | FilterOperator::Lt
                | FilterOperator::Lte
                | FilterOperator::Like
                | FilterOperator::ILike
                | FilterOperator::NotLike
                | FilterOperator::NotILike => types.push(*sql_type),
                // An empty `in` list renders a constant, which binds nothing.
                FilterOperator::In | FilterOperator::NotIn => {
                    if let Some(Value::Array(values)) = &condition.value {
                        types.extend(std::iter::repeat_n(*sql_type, values.len()));
                    }
                }
                FilterOperator::Any | FilterOperator::NotAny => types.push(*sql_type),
                FilterOperator::IsNull | FilterOperator::IsNotNull => {}
                FilterOperator::Between | FilterOperator::NotBetween => {
                    types.extend([*sql_type, *sql_type]);
                }
            },
            Self::Group { filters, .. } => {
                for child in filters {
                    child.collect_param_types(types);
                }
            }
        }
    }
}

/// Whether a comparison renders a null test instead of binding a value.
fn is_absent(value: &Option<Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

/// A read's arguments without its source: what to select, filter, order, and page.
///
/// This is the crate's lowering target: a generated read converts its own options into it and
/// plans the statement, so no caller names this type or hands it a column name.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReadOptions {
    /// The selected columns; empty selects every column.
    pub columns: Vec<SelectColumn>,
    /// The typed filter tree applied to the read.
    pub filters: Vec<TypedFilter>,
    /// The ordering clauses, in order.
    pub sort: Vec<Sort>,
    /// The maximum number of rows.
    pub limit: Option<u64>,
    /// The number of leading rows to skip.
    pub offset: Option<u64>,
}

impl ReadOptions {
    /// The datagrid options for a table, optionally in `schema`.
    #[must_use]
    fn into_query(self, table: &str, schema: Option<&str>) -> QueryOptions {
        QueryOptions {
            source: QuerySource::Table {
                name: table.to_owned(),
                schema: schema.map(str::to_owned),
            },
            columns: self.columns,
            filters: self
                .filters
                .into_iter()
                .map(TypedFilter::into_filter)
                .collect(),
            sort: self.sort,
            limit: self.limit,
            offset: self.offset,
        }
    }

    /// The statement for a table, with every parameter paired with the column type it binds.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Build`] when the options do not form a valid statement, and
    /// [`ReadError::Plan`] when the built parameters and the filters' types disagree.
    pub(crate) fn plan(
        self,
        table: &str,
        schema: Option<&str>,
    ) -> Result<PlannedQuery, super::ReadError> {
        let types: Vec<SqlType> = self
            .filters
            .iter()
            .flat_map(TypedFilter::param_types)
            .collect();
        let built = datagrid_query::build_query(&self.into_query(table, schema))?;
        if types.len() != built.params.len() {
            return Err(super::ReadError::Plan {
                params: built.params.len(),
                types: types.len(),
            });
        }
        Ok(PlannedQuery {
            sql: built.sql,
            params: built
                .params
                .into_iter()
                .zip(types)
                .map(|(value, sql_type)| Param::new(sql_type, value))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use datagrid_query::{Combinator, FilterOperator, Value};

    use super::{Param, ReadOptions, TypedFilter};
    use crate::db::SqlType;

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn plans_a_comparison_with_its_column_type() {
        let options = ReadOptions {
            filters: vec![TypedFilter::condition(
                "tenantId",
                SqlType::Uuid,
                FilterOperator::Eq,
                Some(text("t1")),
            )],
            limit: Some(10),
            ..ReadOptions::default()
        };

        let planned = options.plan("widget", None).expect("the options are valid");

        assert_eq!(
            planned.sql,
            r#"SELECT * FROM "widget" WHERE "tenantId" = $1 LIMIT 10"#
        );
        assert_eq!(planned.params, vec![Param::new(SqlType::Uuid, text("t1"))]);
    }

    #[test]
    fn binds_one_array_parameter_for_any() {
        let options = ReadOptions {
            filters: vec![TypedFilter::condition(
                "id",
                SqlType::Text,
                FilterOperator::Any,
                Some(Value::Array(vec![text("a"), text("b")])),
            )],
            ..ReadOptions::default()
        };

        let planned = options.plan("widget", None).expect("the options are valid");

        assert_eq!(
            planned.sql,
            r#"SELECT * FROM "widget" WHERE "id" = ANY($1)"#
        );
        assert_eq!(planned.params.len(), 1);
        assert_eq!(planned.params[0].sql_type, SqlType::Text);
    }

    #[test]
    fn binds_one_parameter_per_in_element_and_two_for_between() {
        let options = ReadOptions {
            filters: vec![
                TypedFilter::condition(
                    "id",
                    SqlType::Uuid,
                    FilterOperator::In,
                    Some(Value::Array(vec![text("a"), text("b")])),
                ),
                TypedFilter::condition(
                    "createdAt",
                    SqlType::Timestamptz,
                    FilterOperator::Between,
                    Some(Value::Array(vec![text("a"), text("b")])),
                ),
            ],
            ..ReadOptions::default()
        };

        let planned = options.plan("widget", None).expect("the options are valid");

        let types: Vec<SqlType> = planned.params.iter().map(|param| param.sql_type).collect();
        assert_eq!(
            types,
            vec![
                SqlType::Uuid,
                SqlType::Uuid,
                SqlType::Timestamptz,
                SqlType::Timestamptz
            ]
        );
    }

    #[test]
    fn binds_nothing_for_a_null_test() {
        let options = ReadOptions {
            filters: vec![TypedFilter::condition(
                "deletedAt",
                SqlType::Timestamptz,
                FilterOperator::Eq,
                Some(Value::Null),
            )],
            ..ReadOptions::default()
        };

        let planned = options.plan("widget", None).expect("the options are valid");

        assert_eq!(
            planned.sql,
            r#"SELECT * FROM "widget" WHERE "deletedAt" IS NULL"#
        );
        assert!(planned.params.is_empty());
    }

    #[test]
    fn keeps_group_order_in_the_parameter_list() {
        let options = ReadOptions {
            filters: vec![TypedFilter::group(
                Combinator::Or,
                vec![
                    TypedFilter::condition(
                        "a",
                        SqlType::I64,
                        FilterOperator::Eq,
                        Some(Value::Int(1)),
                    ),
                    TypedFilter::condition("b", SqlType::Text, FilterOperator::Eq, Some(text("x"))),
                ],
            )],
            ..ReadOptions::default()
        };

        let planned = options.plan("widget", None).expect("the options are valid");

        assert_eq!(
            planned.sql,
            r#"SELECT * FROM "widget" WHERE ("a" = $1 OR "b" = $2)"#
        );
        assert_eq!(planned.params[0].sql_type, SqlType::I64);
        assert_eq!(planned.params[1].sql_type, SqlType::Text);
    }

    #[test]
    fn binds_nothing_for_a_null_test_inside_a_group() {
        let options = ReadOptions {
            filters: vec![TypedFilter::group(
                Combinator::And,
                vec![TypedFilter::condition(
                    "a",
                    SqlType::I64,
                    FilterOperator::IsNull,
                    None,
                )],
            )],
            ..ReadOptions::default()
        };

        let planned = options
            .plan("widget", None)
            .expect("a null test binds nothing");

        assert!(planned.params.is_empty());
    }
}
