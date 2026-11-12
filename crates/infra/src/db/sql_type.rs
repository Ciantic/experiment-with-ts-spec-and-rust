/// The Postgres type of a column, so a bind value knows how sqlx must encode it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SqlType {
    Bool,
    I16,
    I32,
    I64,
    F32,
    F64,
    Decimal,
    Text,
    Uuid,
    Timestamptz,
    Timestamp,
    Date,
    Time,
    Json,
    Bytea,
}

impl SqlType {
    /// The type name Postgres reports for a value of this type.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::I16 => "int2",
            Self::I32 => "int4",
            Self::I64 => "int8",
            Self::F32 => "float4",
            Self::F64 => "float8",
            Self::Decimal => "decimal",
            Self::Text => "text",
            Self::Uuid => "uuid",
            Self::Timestamptz => "timestamptz",
            Self::Timestamp => "timestamp",
            Self::Date => "date",
            Self::Time => "time",
            Self::Json => "jsonb",
            Self::Bytea => "bytea",
        }
    }
}

impl std::fmt::Display for SqlType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
