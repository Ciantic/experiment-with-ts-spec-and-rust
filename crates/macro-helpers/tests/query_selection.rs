//! The shape `query_selection!` writes, checked against the pieces a generated module supplies.

use macro_helpers::query_selection;

mod common;

use common::{SelectColumn, flat_filter};

query_selection! {
    Widget {
        id,
        tenantId,
        notes,
    }
}

#[test]
fn names_every_selectable_column_in_order() {
    assert_eq!(
        WidgetSelectField::ALL,
        &[
            WidgetSelectField::Id,
            WidgetSelectField::TenantId,
            WidgetSelectField::Notes,
        ]
    );
}

#[test]
fn spells_each_column_as_the_wire_does() {
    assert_eq!(WidgetSelectField::TenantId.as_str(), "tenantId");
    assert_eq!(WidgetSelectField::TenantId.to_string(), "tenantId");
}

#[test]
fn reads_a_column_name_back() {
    assert_eq!(
        WidgetSelectField::from_column("notes"),
        Some(WidgetSelectField::Notes)
    );
    assert_eq!(WidgetSelectField::from_column("total"), None);
}

#[test]
fn builds_the_planner_entry_for_a_column() {
    assert_eq!(
        WidgetSelectField::Id.select(),
        SelectColumn::Name("id".to_owned())
    );
}

#[test]
fn writes_a_selection_as_an_object_naming_each_column() {
    let selection = WidgetSelection(vec![WidgetSelectField::Id, WidgetSelectField::Notes]);

    assert_eq!(
        serde_json::to_value(&selection).expect("a selection writes"),
        serde_json::json!({ "id": true, "notes": true })
    );
}

#[test]
fn writes_an_empty_selection_as_an_empty_object() {
    assert_eq!(
        serde_json::to_value(WidgetSelection::default()).expect("a selection writes"),
        serde_json::json!({})
    );
}

#[test]
fn reads_a_selection_back() {
    let selection: WidgetSelection =
        serde_json::from_value(serde_json::json!({ "tenantId": true })).expect("a selection reads");

    assert_eq!(
        selection,
        WidgetSelection(vec![WidgetSelectField::TenantId])
    );
}

#[test]
fn rejects_a_column_the_entity_does_not_have() {
    let error = serde_json::from_value::<WidgetSelection>(serde_json::json!({ "total": true }))
        .expect_err("an unknown column is rejected");

    assert!(error.to_string().contains("`total`"), "{error}");
}

#[test]
fn collects_the_fields_a_caller_names() {
    let selection: WidgetSelection = [WidgetSelectField::Notes].into_iter().collect();

    assert_eq!(selection, WidgetSelection(vec![WidgetSelectField::Notes]));
}
