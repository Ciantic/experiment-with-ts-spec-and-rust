use serde::de::DeserializeOwned;

use super::read_options::Param;

/// The failure a generated read reports: planning, running, encoding a bind value, or decoding a row.
#[derive(Debug)]
pub enum ReadError {
    /// The read's options do not form a valid statement.
    Build(datagrid_query::QueryError),
    /// The built statement's parameters do not match the filters' declared types.
    Plan {
        /// The number of parameters the builder produced.
        params: usize,
        /// The number of types the filters declared.
        types: usize,
    },
    /// The executor rejected the statement.
    Execute(String),
    /// A model value has no JSON form, so it cannot become a bind value.
    Encode(serde_json::Error),
    /// A model value does not fit the column type it is bound to.
    InvalidValue {
        /// The column type the parameter was bound as.
        sql_type: super::SqlType,
        /// Why the value does not fit.
        detail: String,
    },
    /// A returned column has a type the decoder does not handle.
    Unsupported {
        /// The column the executor returned.
        column: String,
        /// The Postgres type the column carries.
        sql_type: String,
    },
    /// A returned row does not match the entity's selected shape.
    Decode(serde_json::Error),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(error) => write!(f, "invalid read: {error}"),
            Self::Plan { params, types } => {
                write!(
                    f,
                    "read binds {params} parameter(s), but the filters declare {types} type(s)"
                )
            }
            Self::Execute(message) => write!(f, "statement failed: {message}"),
            Self::Encode(error) => write!(f, "bind value encode failed: {error}"),
            Self::InvalidValue { sql_type, detail } => {
                write!(f, "value does not fit column type {sql_type}: {detail}")
            }
            Self::Unsupported { column, sql_type } => {
                write!(f, "column {column:?} has an unsupported type: {sql_type}")
            }
            Self::Decode(error) => write!(f, "row decode failed: {error}"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Build(error) => Some(error),
            Self::Encode(error) | Self::Decode(error) => Some(error),
            Self::Plan { .. }
            | Self::Execute(_)
            | Self::InvalidValue { .. }
            | Self::Unsupported { .. } => None,
        }
    }
}

impl From<datagrid_query::QueryError> for ReadError {
    fn from(error: datagrid_query::QueryError) -> Self {
        Self::Build(error)
    }
}

impl From<serde_json::Error> for ReadError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error)
    }
}

/// One row as an executor returns it: an object keyed by the selected field names.
pub type Row = serde_json::Value;

/// The database port a generated read runs on.
#[allow(async_fn_in_trait)]
pub trait SqlExecutor {
    /// Run one parameterised statement and answer its rows.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Execute`] when the driver rejects the statement.
    async fn query(&self, sql: &str, params: &[Param]) -> Result<Vec<Row>, ReadError>;
}

/// Decode the rows of one read into the entity's selected shape.
///
/// # Errors
///
/// Returns [`ReadError::Decode`] when a row does not match `T`.
pub fn decode_rows<T: DeserializeOwned>(rows: Vec<Row>) -> Result<Vec<T>, ReadError> {
    rows.into_iter()
        .map(|row| serde_json::from_value(row).map_err(ReadError::Decode))
        .collect()
}

#[cfg(test)]
mod tests {
    use models::Partial;
    use serde_json::json;

    use super::{ReadError, Row, decode_rows};

    #[derive(Debug, PartialEq, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct WidgetSelect {
        #[serde(default, skip_serializing_if = "Partial::is_unset")]
        id: Partial<String>,
        #[serde(default, skip_serializing_if = "Partial::is_unset")]
        notes: Partial<Option<String>>,
    }

    #[test]
    fn decodes_a_selected_row_and_leaves_unselected_fields_unset() {
        let rows = vec![json!({ "id": "w1", "notes": null })];

        let decoded = decode_rows::<WidgetSelect>(rows).expect("the rows match the shape");

        assert_eq!(decoded[0].id, Partial::Set("w1".to_owned()));
        assert_eq!(decoded[0].notes, Partial::clear());
    }

    #[test]
    fn reports_a_row_that_does_not_match_the_shape() {
        let rows = vec![json!({ "id": 7 })];

        let error = decode_rows::<WidgetSelect>(rows).expect_err("a number is not an id");

        assert!(matches!(error, ReadError::Decode(_)));
    }

    #[test]
    fn leaves_a_row_of_unselected_fields_empty() {
        let decoded = decode_rows::<WidgetSelect>(vec![Row::Object(serde_json::Map::new())])
            .expect("an empty object decodes");

        assert_eq!(decoded[0].id, Partial::Unset);
    }
}
