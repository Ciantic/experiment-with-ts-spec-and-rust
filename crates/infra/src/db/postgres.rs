use datagrid_query::Value;
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::types::BigDecimal;
use sqlx::types::Uuid;
use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::{Arguments, AssertSqlSafe, Column, PgPool, Row as _, TypeInfo, query_with};

use super::executor::{ReadError, Row, SqlExecutor};
use super::read_options::Param;
use super::sql_type::SqlType;

/// Bind a scalar, or an array of it when the value is one, which is what `any` binds.
macro_rules! bind_as {
    ($arguments:expr, $param:expr, $rust_type:ty, $convert:path) => {{
        let sql_type = $param.sql_type;
        let result = match &$param.value {
            Value::Array(values) => {
                let mut converted: Vec<Option<$rust_type>> = Vec::with_capacity(values.len());
                for value in values {
                    converted.push(match value {
                        Value::Null => None,
                        other => Some($convert(other)?),
                    });
                }
                $arguments.add(converted)
            }
            Value::Null => $arguments.add(Option::<$rust_type>::None),
            other => $arguments.add($convert(other)?),
        };
        result.map_err(|error| ReadError::InvalidValue {
            sql_type,
            detail: error.to_string(),
        })?;
        Ok(())
    }};
}

/// The sqlx-backed driver: one Postgres pool, and reads planned by `datagrid_query`.
impl SqlExecutor for PgPool {
    async fn query(&self, sql: &str, params: &[Param]) -> Result<Vec<Row>, ReadError> {
        let mut arguments = PgArguments::default();
        for param in params {
            bind(&mut arguments, param)?;
        }

        let rows = query_with(AssertSqlSafe(sql), arguments)
            .fetch_all(self)
            .await
            .map_err(|error| ReadError::Execute(error.to_string()))?;

        rows.iter().map(row_to_json).collect()
    }
}

/// Bind one typed parameter, so sqlx encodes it as the column type rather than guessing.
///
/// # Errors
///
/// Returns [`ReadError::InvalidValue`] when the value cannot be represented as the column type.
fn bind(arguments: &mut PgArguments, param: &Param) -> Result<(), ReadError> {
    let sql_type = param.sql_type;
    match sql_type {
        SqlType::Bool => bind_as!(arguments, param, bool, as_bool),
        SqlType::I16 => bind_as!(arguments, param, i16, as_i16),
        SqlType::I32 => bind_as!(arguments, param, i32, as_i32),
        SqlType::I64 => bind_as!(arguments, param, i64, as_i64),
        SqlType::F32 => bind_as!(arguments, param, f32, as_f32),
        SqlType::F64 => bind_as!(arguments, param, f64, as_f64),
        SqlType::Decimal => bind_as!(arguments, param, BigDecimal, as_decimal),
        SqlType::Text => bind_as!(arguments, param, String, as_text),
        SqlType::Uuid => bind_as!(arguments, param, Uuid, as_uuid),
        SqlType::Timestamptz => bind_as!(arguments, param, DateTime<Utc>, as_timestamptz),
        SqlType::Timestamp => bind_as!(arguments, param, NaiveDateTime, as_timestamp),
        SqlType::Date => bind_as!(arguments, param, NaiveDate, as_date),
        SqlType::Time => bind_as!(arguments, param, NaiveTime, as_time),
        SqlType::Json => bind_as!(arguments, param, serde_json::Value, as_json),
        SqlType::Bytea => bind_as!(arguments, param, Vec<u8>, as_bytes),
    }
}

/// Report a value that does not fit its column type.
fn invalid(sql_type: SqlType, value: &Value) -> ReadError {
    ReadError::InvalidValue {
        sql_type,
        detail: format!("unexpected value {value:?}"),
    }
}

/// A value as text, accepting the shapes a caller stores a textual column with.
fn as_text(value: &Value) -> Result<String, ReadError> {
    match value {
        Value::Text(text) => Ok(text.clone()),
        Value::Int(number) => Ok(number.to_string()),
        Value::Float(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        other => Err(invalid(SqlType::Text, other)),
    }
}

fn as_bool(value: &Value) -> Result<bool, ReadError> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::Int(number) => Ok(*number != 0),
        Value::Text(text) => text.parse().map_err(|_| invalid(SqlType::Bool, value)),
        other => Err(invalid(SqlType::Bool, other)),
    }
}

fn as_i64(value: &Value) -> Result<i64, ReadError> {
    match value {
        Value::Int(number) => Ok(*number),
        Value::Float(number) => Ok(*number as i64),
        Value::Text(text) => text.parse().map_err(|_| invalid(SqlType::I64, value)),
        other => Err(invalid(SqlType::I64, other)),
    }
}

fn as_i32(value: &Value) -> Result<i32, ReadError> {
    as_i64(value)?
        .try_into()
        .map_err(|_| invalid(SqlType::I32, value))
}

fn as_i16(value: &Value) -> Result<i16, ReadError> {
    as_i64(value)?
        .try_into()
        .map_err(|_| invalid(SqlType::I16, value))
}

fn as_f64(value: &Value) -> Result<f64, ReadError> {
    match value {
        Value::Float(number) => Ok(*number),
        Value::Int(number) => Ok(*number as f64),
        Value::Text(text) => text.parse().map_err(|_| invalid(SqlType::F64, value)),
        other => Err(invalid(SqlType::F64, other)),
    }
}

fn as_f32(value: &Value) -> Result<f32, ReadError> {
    Ok(as_f64(value)? as f32)
}

fn as_decimal(value: &Value) -> Result<BigDecimal, ReadError> {
    let text = as_text(value)?;
    text.parse().map_err(|_| invalid(SqlType::Decimal, value))
}

fn as_uuid(value: &Value) -> Result<Uuid, ReadError> {
    Uuid::parse_str(&as_text(value)?).map_err(|_| invalid(SqlType::Uuid, value))
}

fn as_timestamptz(value: &Value) -> Result<DateTime<Utc>, ReadError> {
    let text = as_text(value)?;
    if let Ok(parsed) = DateTime::parse_from_rfc3339(&text) {
        return Ok(parsed.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S%.f")
        .map(|naive| DateTime::from_naive_utc_and_offset(naive, Utc))
        .map_err(|_| invalid(SqlType::Timestamptz, value))
}

fn as_timestamp(value: &Value) -> Result<NaiveDateTime, ReadError> {
    Ok(as_timestamptz(value)?.naive_utc())
}

fn as_date(value: &Value) -> Result<NaiveDate, ReadError> {
    NaiveDate::parse_from_str(&as_text(value)?, "%Y-%m-%d")
        .map_err(|_| invalid(SqlType::Date, value))
}

fn as_time(value: &Value) -> Result<NaiveTime, ReadError> {
    NaiveTime::parse_from_str(&as_text(value)?, "%H:%M:%S%.f")
        .map_err(|_| invalid(SqlType::Time, value))
}

fn as_json(value: &Value) -> Result<serde_json::Value, ReadError> {
    match value {
        Value::Json(json) => Ok(json.clone()),
        Value::Text(text) => serde_json::from_str(text).map_err(|_| invalid(SqlType::Json, value)),
        Value::Null => Ok(serde_json::Value::Null),
        Value::Int(number) => Ok(serde_json::Value::from(*number)),
        Value::Float(number) => Ok(serde_json::Value::from(*number)),
        Value::Bool(flag) => Ok(serde_json::Value::from(*flag)),
        Value::Array(values) => values
            .iter()
            .map(as_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        other => Err(invalid(SqlType::Json, other)),
    }
}

fn as_bytes(value: &Value) -> Result<Vec<u8>, ReadError> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        Value::Array(values) => values
            .iter()
            .map(|value| match value {
                Value::Int(number) => {
                    u8::try_from(*number).map_err(|_| invalid(SqlType::Bytea, value))
                }
                other => Err(invalid(SqlType::Bytea, other)),
            })
            .collect(),
        other => Err(invalid(SqlType::Bytea, other)),
    }
}

/// Turn one returned row into the JSON object the selected shape decodes from.
///
/// # Errors
///
/// Returns [`ReadError::Unsupported`] for a column type the decoder does not handle, and
/// [`ReadError::Execute`] when a known column fails to decode.
fn row_to_json(row: &PgRow) -> Result<Row, ReadError> {
    let mut object = serde_json::Map::with_capacity(row.columns().len());
    for (index, column) in row.columns().iter().enumerate() {
        let name = column.name();
        let type_name = column.type_info().name();
        let value = match type_name {
            "BOOL" => opt(decode::<bool>(row, index, name)?),
            "INT2" => opt(decode::<i16>(row, index, name)?),
            "INT4" => opt(decode::<i32>(row, index, name)?),
            "INT8" => opt(decode::<i64>(row, index, name)?),
            "FLOAT4" => opt(decode::<f32>(row, index, name)?),
            "FLOAT8" => opt(decode::<f64>(row, index, name)?),
            "NUMERIC" => {
                opt(decode::<BigDecimal>(row, index, name)?.map(|amount| amount.to_string()))
            }
            "TEXT" | "VARCHAR" | "BPCHAR" | "CHAR" | "NAME" => {
                opt(decode::<String>(row, index, name)?)
            }
            "UUID" => opt(decode::<Uuid>(row, index, name)?.map(|id| id.to_string())),
            "TIMESTAMPTZ" => opt_json(decode::<DateTime<Utc>>(row, index, name)?)?,
            "TIMESTAMP" => opt_json(decode::<NaiveDateTime>(row, index, name)?)?,
            "DATE" => opt_json(decode::<NaiveDate>(row, index, name)?)?,
            "TIME" => opt_json(decode::<NaiveTime>(row, index, name)?)?,
            "JSON" | "JSONB" => {
                decode::<serde_json::Value>(row, index, name)?.unwrap_or(serde_json::Value::Null)
            }
            "BYTEA" => opt_json(decode::<Vec<u8>>(row, index, name)?)?,
            other => {
                return Err(ReadError::Unsupported {
                    column: name.to_owned(),
                    sql_type: other.to_owned(),
                });
            }
        };
        object.insert(name.to_owned(), value);
    }
    Ok(Row::Object(object))
}

/// Read one column as a Rust value; a null column is `None`.
///
/// # Errors
///
/// Returns [`ReadError::Execute`] when the column does not decode as the expected type.
fn decode<T>(row: &PgRow, index: usize, name: &str) -> Result<Option<T>, ReadError>
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(index)
        .map_err(|error| ReadError::Execute(format!("column {name:?}: {error}")))
}

/// A nullable scalar as JSON; a null column stays `null`.
fn opt<T: Into<serde_json::Value>>(value: Option<T>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, Into::into)
}

/// A nullable value with no `Into<serde_json::Value>` hop, such as a timestamp.
///
/// # Errors
///
/// Returns [`ReadError::Decode`] when the value cannot be represented as JSON.
fn opt_json<T: serde::Serialize>(value: Option<T>) -> Result<serde_json::Value, ReadError> {
    match value {
        Some(value) => encode(value),
        None => Ok(serde_json::Value::Null),
    }
}

/// Turn a decoded value into JSON, which is the shape the selected model decodes from.
fn encode<T: serde::Serialize>(value: T) -> Result<serde_json::Value, ReadError> {
    serde_json::to_value(value).map_err(ReadError::Decode)
}

#[cfg(test)]
mod tests {
    use datagrid_query::Value;

    use super::{
        as_bytes, as_date, as_decimal, as_i16, as_i32, as_json, as_text, as_timestamptz, as_uuid,
    };
    use crate::db::ReadError;

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn reads_an_integer_written_as_a_number_or_a_string() {
        assert_eq!(as_i32(&Value::Int(7)).expect("a number fits"), 7);
        assert_eq!(as_i32(&text("7")).expect("a string fits"), 7);
    }

    #[test]
    fn rejects_an_integer_beyond_the_column_range() {
        let error = as_i16(&Value::Int(70_000)).expect_err("an int2 holds at most 32767");

        assert!(matches!(error, ReadError::InvalidValue { .. }));
    }

    #[test]
    fn reads_a_uuid_from_its_canonical_text() {
        let value = text("018f0e2a-6f5b-7c3d-8e1f-2a3b4c5d6e7f");

        assert_eq!(
            as_uuid(&value).expect("the text is a uuid").to_string(),
            "018f0e2a-6f5b-7c3d-8e1f-2a3b4c5d6e7f"
        );
    }

    #[test]
    fn rejects_a_value_that_is_not_a_uuid() {
        assert!(as_uuid(&text("not-a-uuid")).is_err());
    }

    #[test]
    fn reads_a_timestamp_from_rfc3339_or_a_naive_form() {
        let rfc3339 = as_timestamptz(&text("2024-01-02T03:04:05Z")).expect("rfc3339 fits");
        let naive =
            as_timestamptz(&text("2024-01-02T03:04:05")).expect("a naive form is read as utc");

        assert_eq!(rfc3339, naive);
    }

    #[test]
    fn reads_a_date_and_a_decimal_from_text() {
        assert_eq!(
            as_date(&text("2024-01-02"))
                .expect("a date fits")
                .to_string(),
            "2024-01-02"
        );
        assert_eq!(
            as_decimal(&text("12.50"))
                .expect("a decimal fits")
                .to_string(),
            "12.50"
        );
    }

    #[test]
    fn reads_text_from_every_scalar_shape() {
        assert_eq!(as_text(&Value::Int(7)).expect("a number renders"), "7");
        assert_eq!(as_text(&Value::Bool(true)).expect("a flag renders"), "true");
    }

    #[test]
    fn reads_json_from_a_string_that_holds_json() {
        let value = as_json(&text(r#"{"a":1}"#)).expect("the text is json");

        assert_eq!(value["a"], 1);
    }

    #[test]
    fn reads_bytes_from_an_array_of_numbers() {
        let value = Value::Array(vec![Value::Int(1), Value::Int(255)]);

        assert_eq!(as_bytes(&value).expect("bytes fit"), vec![1, 255]);
    }
}
