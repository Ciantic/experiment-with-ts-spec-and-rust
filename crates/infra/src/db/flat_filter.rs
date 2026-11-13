//! The flat `column__operator` shape a filter entry takes on the wire. See docs/rust-queries.md.
use std::collections::BTreeMap;

use datagrid_query::Combinator;
use serde::de::Error as _;
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// How a group combines its branches: all of them, or any one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    /// Every branch must match.
    And,
    /// Any branch may match.
    Or,
}

impl FilterKind {
    /// The planner's combinator for this group kind.
    #[must_use]
    pub const fn combinator(self) -> Combinator {
        match self {
            Self::And => Combinator::And,
            Self::Or => Combinator::Or,
        }
    }
}

/// Write one `column__operator` entry: a map holding the single key and its value.
///
/// # Errors
///
/// Returns the serializer's error when the value cannot be written.
pub(crate) fn serialize<S: Serializer, T: Serialize>(
    key: &str,
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry(key, value)?;
    map.end()
}

/// Read one `column__operator` entry, and hand its key and value to `parse`.
///
/// # Errors
///
/// Returns the deserializer's error when the input is not one entry, or `parse` rejects it.
pub(crate) fn deserialize<'de, D: Deserializer<'de>, T>(
    deserializer: D,
    parse: impl FnOnce(&str, Value) -> Result<T, String>,
) -> Result<T, D::Error> {
    let entries = <BTreeMap<String, Value> as Deserialize>::deserialize(deserializer)?;
    let mut entries = entries.into_iter();
    match (entries.next(), entries.next()) {
        (Some((key, value)), None) => parse(&key, value).map_err(D::Error::custom),
        _ => Err(D::Error::custom(
            "a filter entry names exactly one `column__operator` comparison",
        )),
    }
}

/// Read the payload of a flat entry, as the operator's variant carries it.
///
/// # Errors
///
/// Returns a message naming the expected form when the value does not fit.
pub(crate) fn payload<T: serde::de::DeserializeOwned>(
    key: &str,
    value: Value,
) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|error| format!("`{key}` needs a value of its operator's type: {error}"))
}

/// Read a null test's payload, which is `true`; the test names its column, so the value carries nothing else.
///
/// # Errors
///
/// Returns a message naming the key when the payload is not `true`.
pub(crate) fn null_test(key: &str, value: Value) -> Result<(), String> {
    match value {
        Value::Bool(true) => Ok(()),
        _ => Err(format!("`{key}` needs `true`")),
    }
}

/// Read a selection: an object naming each selected column, so `field` looks the name up.
///
/// # Errors
///
/// Returns a message naming the column when it is unknown, or its value is not `true`.
pub(crate) fn selection<'de, D, F, N>(deserializer: D, field: F) -> Result<Vec<N>, D::Error>
where
    D: Deserializer<'de>,
    F: Fn(&str) -> Option<N>,
{
    let entries = <BTreeMap<String, bool> as Deserialize>::deserialize(deserializer)?;
    let mut names = Vec::with_capacity(entries.len());
    for (column, selected) in entries {
        let name = field(&column)
            .ok_or_else(|| D::Error::custom(format!("unknown select field `{column}`")))?;
        if !selected {
            return Err(D::Error::custom(format!(
                "select `{column}` must be `true`"
            )));
        }
        names.push(name);
    }
    Ok(names)
}

/// Read an ordering clause: a `[column, direction]` tuple, so `field` looks the column up.
///
/// # Errors
///
/// Returns a message naming the column when it is unknown, or the tuple's direction does not fit.
pub(crate) fn sort<'de, D, F, N, T>(deserializer: D, field: F) -> Result<(N, T), D::Error>
where
    D: Deserializer<'de>,
    F: Fn(&str) -> Option<N>,
    T: Deserialize<'de>,
{
    let (column, direction) = <(String, T) as Deserialize>::deserialize(deserializer)?;
    let name = field(&column)
        .ok_or_else(|| D::Error::custom(format!("unknown order field `{column}`")))?;
    Ok((name, direction))
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    /// One comparison, as a generated column filter writes itself.
    #[derive(Debug, PartialEq)]
    enum TenantFilter {
        Eq(String),
        Any(Vec<String>),
    }

    impl Serialize for TenantFilter {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match self {
                Self::Eq(value) => super::serialize("tenantId__eq", value, serializer),
                Self::Any(values) => super::serialize("tenantId__any", values, serializer),
            }
        }
    }

    impl<'de> Deserialize<'de> for TenantFilter {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            super::deserialize(deserializer, |key, value| match key {
                "tenantId__eq" => Ok(Self::Eq(super::payload(key, value)?)),
                "tenantId__any" => Ok(Self::Any(super::payload(key, value)?)),
                other => Err(format!("unknown filter `{other}`")),
            })
        }
    }

    #[test]
    fn writes_one_flat_key() {
        assert_eq!(
            serde_json::to_value(TenantFilter::Eq("t1".to_owned())).expect("it encodes"),
            serde_json::json!({ "tenantId__eq": "t1" })
        );
        assert_eq!(
            serde_json::to_value(TenantFilter::Any(vec!["a".to_owned()])).expect("it encodes"),
            serde_json::json!({ "tenantId__any": ["a"] })
        );
    }

    #[test]
    fn reads_one_flat_key() {
        let filter: TenantFilter =
            serde_json::from_value(serde_json::json!({ "tenantId__any": ["a"] }))
                .expect("it decodes");

        assert_eq!(filter, TenantFilter::Any(vec!["a".to_owned()]));
    }

    #[test]
    fn rejects_an_unknown_operator() {
        let result: Result<TenantFilter, _> =
            serde_json::from_value(serde_json::json!({ "tenantId__gte": "t1" }));

        assert!(result.is_err());
    }

    #[test]
    fn rejects_two_keys() {
        let result: Result<TenantFilter, _> = serde_json::from_value(
            serde_json::json!({ "tenantId__eq": "t1", "tenantId__any": [] }),
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_no_key() {
        let result: Result<TenantFilter, _> = serde_json::from_value(serde_json::json!({}));

        assert!(result.is_err());
    }

    /// One ordering direction, as the planner spells it.
    #[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum Direction {
        Asc,
        Desc,
    }

    #[test]
    fn reads_a_null_test_of_true() {
        assert!(super::null_test("deletedAt__isNull", serde_json::json!(true)).is_ok());
    }

    #[test]
    fn rejects_a_null_test_of_anything_else() {
        for value in [
            serde_json::json!(false),
            serde_json::json!(null),
            serde_json::json!(""),
        ] {
            assert!(super::null_test("deletedAt__isNull", value).is_err());
        }
    }

    #[test]
    fn reads_a_selection_into_the_columns_it_names() {
        let columns: Vec<&str> = super::selection(
            serde_json::json!({ "id": true, "number": true }),
            |column| match column {
                "id" => Some("id"),
                "number" => Some("number"),
                _ => None,
            },
        )
        .expect("it decodes");

        assert_eq!(columns, ["id", "number"]);
    }

    #[test]
    fn rejects_a_selected_column_that_is_not_true() {
        let result: Result<Vec<&str>, _> =
            super::selection(serde_json::json!({ "id": false }), |column| {
                (column == "id").then_some("id")
            });

        assert!(result.is_err());
    }

    #[test]
    fn rejects_an_unknown_selected_column() {
        let result: Result<Vec<&str>, _> =
            super::selection(serde_json::json!({ "hidden": true }), |_| None);

        assert!(result.is_err());
    }

    #[test]
    fn reads_an_ordering_clause_into_its_column_and_direction() {
        let clause: (&str, Direction) =
            super::sort(serde_json::json!(["createdAt", "desc"]), |column| {
                (column == "createdAt").then_some("createdAt")
            })
            .expect("it decodes");

        assert_eq!(clause, ("createdAt", Direction::Desc));
    }

    #[test]
    fn rejects_an_ordering_clause_without_a_direction() {
        let result: Result<(&str, Direction), _> =
            super::sort(serde_json::json!(["createdAt"]), |_| Some("createdAt"));

        assert!(result.is_err());
    }

    #[test]
    fn reads_a_selection_in_the_order_its_entries_come_back_in() {
        let columns: Vec<&str> = super::selection(
            serde_json::json!({ "number": true, "id": true }),
            |column| match column {
                "id" => Some("id"),
                "number" => Some("number"),
                _ => None,
            },
        )
        .expect("it decodes");

        assert_eq!(columns, ["id", "number"]);
    }

    #[test]
    fn rejects_an_unknown_ordered_column() {
        let result: Result<(&str, Direction), _> =
            super::sort(serde_json::json!(["hidden", "asc"]), |_| None);

        assert!(result.is_err());
    }
}
