//! The shape `query_order!` writes, checked against the pieces a generated module supplies.

use macro_helpers::query_order;

mod common;

use common::{Sort, SortDirection, flat_filter};

query_order! {
    Widget {
        createdAt,
        updatedAt,
    }
}

#[test]
fn names_every_orderable_column_in_order() {
    assert_eq!(
        WidgetOrderField::ALL,
        &[WidgetOrderField::CreatedAt, WidgetOrderField::UpdatedAt]
    );
}

#[test]
fn spells_each_column_as_the_wire_does() {
    assert_eq!(WidgetOrderField::CreatedAt.as_str(), "createdAt");
    assert_eq!(WidgetOrderField::CreatedAt.to_string(), "createdAt");
}

#[test]
fn reads_a_column_name_back() {
    assert_eq!(
        WidgetOrderField::from_column("updatedAt"),
        Some(WidgetOrderField::UpdatedAt)
    );
    assert_eq!(WidgetOrderField::from_column("notes"), None);
}

#[test]
fn builds_a_clause_in_either_direction() {
    assert_eq!(
        WidgetOrderField::CreatedAt.desc().into_sort(),
        Sort {
            column: "createdAt".to_owned(),
            direction: SortDirection::Desc,
            nulls: None,
        }
    );
    assert_eq!(
        WidgetOrderField::CreatedAt.asc().into_sort().direction,
        SortDirection::Asc
    );
}

#[test]
fn writes_a_clause_as_a_column_and_direction_tuple() {
    let order = WidgetOrderField::UpdatedAt.desc();

    assert_eq!(
        serde_json::to_value(&order).expect("a clause writes"),
        serde_json::json!(["updatedAt", "desc"])
    );
}

#[test]
fn reads_a_clause_back() {
    let order: WidgetOrder =
        serde_json::from_value(serde_json::json!(["createdAt", "asc"])).expect("a clause reads");

    assert_eq!(order, WidgetOrderField::CreatedAt.asc());
}

#[test]
fn rejects_a_column_the_entity_cannot_order_by() {
    let error = serde_json::from_value::<WidgetOrder>(serde_json::json!(["notes", "asc"]))
        .expect_err("an unknown column is rejected");

    assert!(error.to_string().contains("`notes`"), "{error}");
}
