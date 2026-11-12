use datagrid_query::Value;

use super::ReadError;

/// A value a filter can bind: anything with a JSON form, which includes every model newtype such
/// as a branded id or `Money`.
pub trait BindValue {
    /// The builder's bind value.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Encode`] when the value has no JSON form.
    fn to_bind_value(&self) -> Result<Value, ReadError>;
}

impl<T: serde::Serialize + ?Sized> BindValue for T {
    fn to_bind_value(&self) -> Result<Value, ReadError> {
        let json = serde_json::to_value(self).map_err(ReadError::Encode)?;
        serde_json::from_value(json).map_err(ReadError::Encode)
    }
}

/// Turn a model value into the builder's bind value.
///
/// # Errors
///
/// Returns [`ReadError::Encode`] when the value has no JSON form.
pub fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ReadError> {
    value.to_bind_value()
}

#[cfg(test)]
mod tests {
    use datagrid_query::Value;

    use super::{BindValue, to_value};

    /// The shape a generated model id takes: a newtype over its stored value.
    #[derive(Debug, serde::Serialize)]
    struct WidgetId(String);

    #[test]
    fn binds_a_model_newtype_as_its_inner_value() {
        let id = WidgetId("w1".to_owned());

        assert_eq!(
            id.to_bind_value().expect("an id encodes"),
            Value::Text("w1".to_owned())
        );
    }

    #[test]
    fn binds_a_borrowed_value() {
        let id = WidgetId("w1".to_owned());
        let borrowed: &WidgetId = &id;

        assert_eq!(
            borrowed.to_bind_value().expect("a borrow encodes"),
            Value::Text("w1".to_owned())
        );
    }

    #[test]
    fn binds_the_scalars_a_column_holds() {
        assert_eq!(
            7_i64.to_bind_value().expect("an integer encodes"),
            Value::Int(7)
        );
        assert_eq!(
            true.to_bind_value().expect("a flag encodes"),
            Value::Bool(true)
        );
        assert_eq!(
            "x".to_bind_value().expect("a string encodes"),
            Value::Text("x".to_owned())
        );
        assert_eq!(
            (None::<String>).to_bind_value().expect("a null encodes"),
            Value::Null
        );
    }

    #[test]
    fn binds_a_serialized_struct_as_json() {
        #[derive(serde::Serialize)]
        struct Shape {
            sides: u8,
        }

        let value = Shape { sides: 4 }
            .to_bind_value()
            .expect("a struct encodes");

        assert_eq!(value, Value::Json(serde_json::json!({ "sides": 4 })));
    }

    #[test]
    fn keeps_the_free_function_as_a_wrapper() {
        assert_eq!(
            to_value(&WidgetId("w1".to_owned())).expect("an id encodes"),
            Value::Text("w1".to_owned())
        );
    }
}
