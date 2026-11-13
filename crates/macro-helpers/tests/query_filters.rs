//! The shape `query_filters!` writes, checked against the pieces a generated module supplies.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use macro_helpers::query_filters;

mod common;

use common::{
    Combinator, FilterKind, FilterOperator, ReadError, SqlType, TypedFilter, flat_filter,
};

query_filters! {
    Widget {
        id: u64, Uuid, [eq],
        tenantId: u64, Uuid, [eq],
        score: i64, Integer, [gte, any, between, isNull],
    }
}

fn json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("a filter has a JSON form")
}

fn id() -> WidgetIdFilter {
    WidgetIdFilter::Eq(7)
}

fn round_trip<T: DeserializeOwned + fmt::Debug + PartialEq + Serialize>(value: &T) -> T {
    let written = serde_json::to_string(value).expect("a filter writes");
    serde_json::from_str(&written).expect("a filter reads")
}

#[test]
fn writes_one_comparison_as_its_flat_entry() {
    assert_eq!(json(&id()), serde_json::json!({ "id__eq": 7 }));
}

#[test]
fn writes_a_list_as_the_array_its_operator_takes() {
    assert_eq!(
        json(&WidgetScoreFilter::Any(vec![1, 2])),
        serde_json::json!({ "score__any": [1, 2] })
    );
}

#[test]
fn writes_a_range_as_a_two_element_array() {
    assert_eq!(
        json(&WidgetScoreFilter::Between(1, 2)),
        serde_json::json!({ "score__between": [1, 2] })
    );
}

#[test]
fn writes_a_null_test_as_true() {
    assert_eq!(
        json(&WidgetScoreFilter::IsNull),
        serde_json::json!({ "score__isNull": true })
    );
}

#[test]
fn reads_back_every_shape_it_writes() {
    assert_eq!(round_trip(&id()), id());
    for filter in [
        WidgetScoreFilter::Gte(1),
        WidgetScoreFilter::Any(vec![1, 2]),
        WidgetScoreFilter::Between(1, 2),
        WidgetScoreFilter::IsNull,
    ] {
        assert_eq!(round_trip(&filter), filter);
    }
}

#[test]
fn names_a_camel_case_column_in_upper_camel() {
    assert_eq!(
        json(&WidgetTenantIdFilter::Eq(1)),
        serde_json::json!({ "tenantId__eq": 1 })
    );
}

#[test]
fn rejects_a_null_test_payload_that_is_not_true() {
    let error = serde_json::from_str::<WidgetScoreFilter>(r#"{"score__isNull":false}"#)
        .expect_err("a false payload is rejected");

    assert!(error.to_string().contains("score__isNull"), "{error}");
}

#[test]
fn names_the_entity_and_column_when_the_key_is_unknown() {
    let error = serde_json::from_str::<WidgetIdFilter>(r#"{"id__like":"x"}"#)
        .expect_err("an unlisted operator is rejected");

    assert!(
        error
            .to_string()
            .contains("unknown filter `id__like` on `Widget.id`"),
        "{error}"
    );
}

#[test]
fn lowers_each_comparison_to_its_column_type() {
    assert_eq!(
        WidgetScoreFilter::Gte(3)
            .into_typed_filter()
            .expect("a value encodes"),
        TypedFilter::One {
            column: "score",
            sql: SqlType::Integer,
            operator: FilterOperator::Gte,
            value: serde_json::json!(3),
        }
    );
    assert_eq!(
        WidgetScoreFilter::Any(vec![1, 2])
            .into_typed_filter()
            .expect("a list encodes"),
        TypedFilter::List {
            column: "score",
            sql: SqlType::Integer,
            operator: FilterOperator::Any,
            values: serde_json::json!([1, 2]),
        }
    );
    assert_eq!(
        WidgetScoreFilter::Between(1, 2)
            .into_typed_filter()
            .expect("a pair encodes"),
        TypedFilter::Pair {
            column: "score",
            sql: SqlType::Integer,
            operator: FilterOperator::Between,
            lower: serde_json::json!(1),
            upper: serde_json::json!(2),
        }
    );
    assert_eq!(
        WidgetScoreFilter::IsNull
            .into_typed_filter()
            .expect("a null test binds nothing"),
        TypedFilter::NullTest {
            column: "score",
            sql: SqlType::Integer,
            operator: FilterOperator::IsNull,
        }
    );
}

#[test]
fn writes_a_column_filter_as_the_entity_entry_it_is() {
    let filter = WidgetFilter::from(WidgetIdFilter::Eq(7));

    assert_eq!(json(&filter), serde_json::json!({ "id__eq": 7 }));
}

#[test]
fn writes_an_or_group_with_the_kind_discriminator() {
    let filter = WidgetFilter::Group {
        kind: FilterKind::Or,
        filters: vec![
            WidgetScoreFilter::Gte(1).into(),
            WidgetScoreFilter::Gte(2).into(),
        ],
    };

    assert_eq!(
        json(&filter),
        serde_json::json!({
            "kind": "or",
            "filters": [{ "score__gte": 1 }, { "score__gte": 2 }],
        })
    );
}

#[test]
fn reads_back_the_entries_and_the_group_it_writes() {
    let entries = serde_json::json!([{ "id__eq": 7 }, { "score__isNull": true }]);
    let filters: Vec<WidgetFilter> = serde_json::from_value(entries).expect("entries read");

    assert_eq!(
        filters,
        vec![
            WidgetIdFilter::Eq(7).into(),
            WidgetScoreFilter::IsNull.into(),
        ]
    );

    let group = serde_json::json!({
        "kind": "and",
        "filters": [{ "score__gte": 1 }, { "id__eq": 7 }],
    });
    assert_eq!(
        serde_json::from_value::<WidgetFilter>(group).expect("a group reads"),
        WidgetFilter::Group {
            kind: FilterKind::And,
            filters: vec![
                WidgetScoreFilter::Gte(1).into(),
                WidgetIdFilter::Eq(7).into(),
            ],
        }
    );
}

#[test]
fn lowers_a_column_filter_as_its_one_entry() {
    let typed = WidgetFilter::from(WidgetIdFilter::Eq(7))
        .into_typed_filters()
        .expect("an id encodes");

    assert_eq!(
        typed,
        vec![TypedFilter::One {
            column: "id",
            sql: SqlType::Uuid,
            operator: FilterOperator::Eq,
            value: serde_json::json!(7),
        }]
    );
}

#[test]
fn keeps_a_group_of_two_as_its_own_combinator() {
    let typed = WidgetFilter::Group {
        kind: FilterKind::Or,
        filters: vec![
            WidgetScoreFilter::Gte(1).into(),
            WidgetScoreFilter::Gte(2).into(),
        ],
    }
    .into_typed_filters()
    .expect("the branches encode");

    assert!(matches!(
        typed.as_slice(),
        [TypedFilter::Group {
            combinator: Combinator::Or,
            filters,
        }] if filters.len() == 2
    ));
}

#[test]
fn unwraps_a_group_of_one_and_empties_a_group_of_none() {
    let one = WidgetFilter::Group {
        kind: FilterKind::And,
        filters: vec![WidgetScoreFilter::Gte(1).into()],
    }
    .into_typed_filters()
    .expect("the branch encodes");

    assert!(matches!(one.as_slice(), [TypedFilter::One { .. }]));
    assert_eq!(
        WidgetFilter::Group {
            kind: FilterKind::And,
            filters: Vec::new(),
        }
        .into_typed_filters()
        .expect("an empty group binds nothing"),
        Vec::new()
    );
}
