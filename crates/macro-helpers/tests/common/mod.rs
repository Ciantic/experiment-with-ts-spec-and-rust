//! The pieces a generated query module supplies at its macro call sites: the planner's types and
//! the wire helpers `crate::db::flat_filter` provides. Each test binary uses a subset.
#![allow(dead_code)]

use std::fmt;

use serde::Serialize;
use serde_json::Value;

/// The planner's error, as far as a lowering can carry one.
#[derive(Debug, PartialEq)]
pub struct ReadError(String);

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A column's SQL type, as far as a lowering names one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SqlType {
    Uuid,
    Integer,
    Timestamptz,
}

/// A comparison's operator, as far as a lowering names one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterOperator {
    Eq,
    Gte,
    Any,
    Between,
    IsNull,
}

/// One typed filter, as far as a lowering builds one.
#[derive(Clone, Debug, PartialEq)]
pub enum TypedFilter {
    One {
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
        value: Value,
    },
    List {
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
        values: Value,
    },
    Pair {
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
        lower: Value,
        upper: Value,
    },
    NullTest {
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
    },
    Group {
        combinator: Combinator,
        filters: Vec<Self>,
    },
}

/// How a group combines its branches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Combinator {
    And,
    Or,
}

/// One select-list entry, as far as a generated field names one.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectColumn {
    Name(String),
}

/// A column's direction, as far as a generated clause names one.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

/// One ordering clause, as far as a generated clause builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Sort {
    pub column: String,
    pub direction: SortDirection,
    pub nulls: Option<()>,
}

/// Whether every branch of a group must match or any may.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterKind {
    And,
    Or,
}

impl FilterKind {
    pub fn combinator(self) -> Combinator {
        match self {
            Self::And => Combinator::And,
            Self::Or => Combinator::Or,
        }
    }
}

impl TypedFilter {
    pub fn group(combinator: Combinator, filters: Vec<Self>) -> Self {
        Self::Group {
            combinator,
            filters,
        }
    }

    pub fn one<T: Serialize>(
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
        value: T,
    ) -> Result<Self, ReadError> {
        Ok(Self::One {
            column,
            sql,
            operator,
            value: encodes(&value)?,
        })
    }

    pub fn list<T: Serialize>(
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
        values: Vec<T>,
    ) -> Result<Self, ReadError> {
        Ok(Self::List {
            column,
            sql,
            operator,
            values: encodes(&values)?,
        })
    }

    pub fn pair<T: Serialize>(
        column: &'static str,
        sql: SqlType,
        operator: FilterOperator,
        lower: T,
        upper: T,
    ) -> Result<Self, ReadError> {
        Ok(Self::Pair {
            column,
            sql,
            operator,
            lower: encodes(&lower)?,
            upper: encodes(&upper)?,
        })
    }

    pub fn null_test(column: &'static str, sql: SqlType, operator: FilterOperator) -> Self {
        Self::NullTest {
            column,
            sql,
            operator,
        }
    }
}

/// The JSON form of a bound value, or the error a lowering carries when it has none.
pub fn encodes<T: Serialize>(value: &T) -> Result<Value, ReadError> {
    serde_json::to_value(value).map_err(|error| ReadError(error.to_string()))
}

/// The wire helpers a generated module imports, with the same shapes `infra` provides.
pub mod flat_filter {
    use std::collections::BTreeMap;

    use serde::de::DeserializeOwned;
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer, T: Serialize>(
        key: &str,
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(key, value)?;
        map.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T>(
        deserializer: D,
        parse: impl FnOnce(&str, Value) -> Result<T, String>,
    ) -> Result<T, D::Error> {
        let entries = <BTreeMap<String, Value> as Deserialize>::deserialize(deserializer)?;
        let mut entries = entries.into_iter();
        match (entries.next(), entries.next()) {
            (Some((key, value)), None) => parse(&key, value).map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom(
                "a filter entry names exactly one `column__operator` comparison",
            )),
        }
    }

    pub fn payload<T: DeserializeOwned>(key: &str, value: Value) -> Result<T, String> {
        serde_json::from_value(value)
            .map_err(|error| format!("`{key}` needs a value of its operator's type: {error}"))
    }

    pub fn null_test(key: &str, value: Value) -> Result<(), String> {
        match value {
            Value::Bool(true) => Ok(()),
            _ => Err(format!("`{key}` needs `true`")),
        }
    }

    /// A column's ordering, read as the two-element tuple a clause is written as.
    pub fn sort<'de, D, F, T>(
        deserializer: D,
        field: F,
    ) -> Result<(T, super::SortDirection), D::Error>
    where
        D: Deserializer<'de>,
        F: FnOnce(&str) -> Option<T>,
    {
        let (column, direction) =
            <(String, super::SortDirection) as Deserialize>::deserialize(deserializer)?;
        match field(&column) {
            Some(field) => Ok((field, direction)),
            None => Err(serde::de::Error::custom(format!(
                "`{column}` is not an orderable column"
            ))),
        }
    }

    /// A selection, read as an object naming each selected column.
    pub fn selection<'de, D, F, N>(deserializer: D, field: F) -> Result<Vec<N>, D::Error>
    where
        D: Deserializer<'de>,
        F: Fn(&str) -> Option<N>,
    {
        let selected = <BTreeMap<String, bool> as Deserialize>::deserialize(deserializer)?;
        let mut fields = Vec::new();
        for (column, chosen) in selected {
            if !chosen {
                return Err(serde::de::Error::custom(format!("`{column}` needs `true`")));
            }
            match field(&column) {
                Some(selected) => fields.push(selected),
                None => {
                    return Err(serde::de::Error::custom(format!(
                        "`{column}` is not a selectable column"
                    )));
                }
            }
        }
        Ok(fields)
    }
}
