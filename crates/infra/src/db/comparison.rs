use serde::{Deserialize, Serialize};

/// A comparison on one column, whose value type is the column's model type.
///
/// The type parameter is what makes a filter payload safe: `Comparison<TenantId>` only
/// deserializes a value a `TenantId` accepts, so a value of the wrong type is a decode error
/// rather than a statement that fails later. See `docs/rust-queries.md`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Comparison<T> {
    /// `column = value`.
    Eq(T),
    /// `column <> value`.
    Ne(T),
    /// `column = any(values)`, bound as one array parameter.
    Any(Vec<T>),
    /// `column is null`.
    IsNull,
    /// `column is not null`.
    IsNotNull,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::Comparison;

    /// The shape a generated model id takes: a newtype over its stored value.
    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    struct TenantId(String);

    #[test]
    fn serializes_as_the_operator_name() {
        assert_eq!(
            serde_json::to_value(Comparison::Eq(TenantId("t1".to_owned()))).expect("it encodes"),
            json!({ "eq": "t1" })
        );
        assert_eq!(
            serde_json::to_value(Comparison::<TenantId>::IsNull).expect("it encodes"),
            json!("isNull")
        );
        assert_eq!(
            serde_json::to_value(Comparison::<TenantId>::IsNotNull).expect("it encodes"),
            json!("isNotNull")
        );
    }

    #[test]
    fn rejects_a_value_the_column_type_does_not_accept() {
        assert!(serde_json::from_value::<Comparison<TenantId>>(json!({ "eq": true })).is_err());
        assert!(serde_json::from_value::<Comparison<TenantId>>(json!({ "eq": 7 })).is_err());
        assert!(serde_json::from_value::<Comparison<TenantId>>(json!({ "eq": null })).is_err());
    }

    #[test]
    fn accepts_a_value_the_column_type_accepts() {
        assert_eq!(
            serde_json::from_value::<Comparison<TenantId>>(json!({ "eq": "t1" }))
                .expect("a string fits"),
            Comparison::Eq(TenantId("t1".to_owned()))
        );
        assert_eq!(
            serde_json::from_value::<Comparison<TenantId>>(json!({ "any": ["a", "b"] }))
                .expect("strings fit"),
            Comparison::Any(vec![TenantId("a".to_owned()), TenantId("b".to_owned())])
        );
        assert_eq!(
            serde_json::from_value::<Comparison<TenantId>>(json!("isNull"))
                .expect("a null test fits"),
            Comparison::IsNull
        );
    }

    #[test]
    fn rejects_an_operator_a_filter_field_does_not_offer() {
        assert!(serde_json::from_value::<Comparison<TenantId>>(json!({ "gt": "t1" })).is_err());
        assert!(
            serde_json::from_value::<Comparison<TenantId>>(json!({ "between": ["a", "b"] }))
                .is_err()
        );
    }

    #[test]
    fn round_trips_through_json() {
        let comparison = Comparison::Any(vec![TenantId("a".to_owned())]);

        let json = serde_json::to_string(&comparison).expect("it encodes");
        let back: Comparison<TenantId> = serde_json::from_str(&json).expect("it decodes");

        assert_eq!(back, comparison);
    }
}
