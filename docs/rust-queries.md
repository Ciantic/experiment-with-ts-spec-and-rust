# Rust queries

`scripts/infra/generate-queries-rs.ts` writes one read module per entity into
`crates/infra/src/db/queries/`. An entity with `@queries query` at
`spec/src/widget-service/Widget.ts` gets
`crates/infra/src/db/queries/query_widget.rs`. The crate writes no SQL of its
own: every read is built by `datagrid_query`, so Rust and the TypeScript side
share one statement shape.

## The generated module

```rust
pub fn build_query_widget(options: WidgetReadOptions) -> Result<PlannedQuery, ReadError> { … }

pub async fn query_widget(
    db: &impl SqlExecutor,
    options: WidgetReadOptions,
) -> Result<Vec<WidgetSelect>, ReadError> { … }
```

The read takes the entity's own options, `WidgetReadOptions`, so its selection
and ordering can only name `Widget`'s columns. The generic `ReadOptions` is not
part of the generated surface; the entity options convert to it internally.

`query_widget` runs the statement and decodes every row into the model's
selected shape, `WidgetSelect`, from `crates/models` (`docs/rust-models.md`).
A field the read did not select stays `Partial::Unset`, so the caller can tell
"not read" from "null". `build_query_widget` is the same read without the
database: a `PlannedQuery`, whose SQL and typed parameters a caller can hand to
a driver itself.

## Typed fields

Each module names the entity's columns once, as one enum per capability:

| Type | Built from | Fields |
| --- | --- | --- |
| `WidgetSelectField` | every column | selectable |
| `WidgetFilterField` | `@queryFilter` | filterable, with `WidgetFilter` as the filter it builds |
| `WidgetOrderField` | `@queryOrderBy` | orderable, with `WidgetOrder` as the clause it builds |

A column an `@inlined` branch reaches through that branch is not selectable by
name, so it is absent from `WidgetSelectField`. An entity that declares no
filter or ordering column gets the matching enum with no variants, so a caller
cannot name one.

Every enum carries `ALL`, `as_str` and `Display`, plus the helpers its
capability needs:

```rust
WidgetSelectField::Notes.select();                          // SelectColumn::Name("notes")
WidgetFilterField::TenantId.eq(&tenant_id)?;                // "tenantId" = $1, a TenantId directly
WidgetFilterField::Id.any([first_id, second_id])?;          // "id" = ANY($1), one array parameter
WidgetFilterField::DeletedAt.is_null();                     // "deletedAt" is null
WidgetOrderField::CreatedAt.desc();                         // "createdAt" DESC
```

`condition(operator, value)` is the general form the others wrap. Every filter
helper returns the entity's own filter type, `WidgetFilter`, so a filter always
carries the column it targets; `sql_type()` maps that field to the type the
driver binds. `WidgetFilter::and(…)` and `WidgetFilter::or(…)` nest filters,
and each child keeps its own column type.

```rust
WidgetFilter::or([
    WidgetFilterField::Number.is_null(),
    WidgetFilterField::SupplierId.is_not_null(),
])
```

## Values

A filter takes the model's own type, not a converted value. `eq`, `ne`,
`condition`, `any`, and `any_of` accept anything that has a JSON form
(`BindValue`), which covers every generated newtype — `TenantId`, `Money`,
`Version`, a `CustomerId` — as well as the scalars a column holds:

```rust
WidgetFilterField::TenantId.eq(&tenant_id)?;                 // a borrow of a branded id
WidgetFilterField::Amount.eq(Money(decimal))?;               // a primitive newtype
WidgetFilterField::Id.any([first, second])?;                 // an iterator of ids
WidgetFilterField::Id.any_of(model.ids)?;                    // a Vec<Id> as one parameter
WidgetFilterField::Notes.eq("draft")?;                       // a plain scalar
```

A newtype serializes as the value it wraps, so `TenantId(String)` binds its
string and `Version(i64)` binds its integer. The conversion can only fail when a
value has no JSON form, which is why the value-carrying helpers return
`Result<WidgetFilter, ReadError>`; the null tests bind nothing and return a
`WidgetFilter` directly. `Condition` also keeps a raw constructor,
`WidgetFilter::condition(field, operator, Some(value))`, for a value that is
already a `datagrid_query::Value`.

The field enums are the validity check: a column the spec does not expose
cannot be named, so a caller never builds a filter or an ordering the entity
forbids. A raw column name has no route into a generated read.

## Read options

Each generated module declares the entity's read options, whose fields are the
entity's own typed sets:

```rust
pub struct WidgetReadOptions {
    pub select: Vec<WidgetSelectField>,
    pub filters: Vec<TypedFilter>,
    pub order: Vec<WidgetOrder>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}
```

An empty `select` selects every column. A `WidgetFilter` pairs a
`WidgetFilterField` with an operator and a value, either directly or inside an
`and` / `or` group. A `WidgetOrder` pairs a `WidgetOrderField` with a direction
and an optional nulls order. Both types only mention the entity's own columns,
so an options value cannot name a column the spec forbids.

The options are built either literally or fluently:

```rust
let options = WidgetReadOptions::with_select([WidgetSelectField::Id, WidgetSelectField::Notes])
    .filter(WidgetFilterField::TenantId.eq(&tenant_id)?)
    .order(WidgetOrderField::CreatedAt.desc().nulls_last())
    .limit(50);
```

`into_read_options` lowers them to the source-independent `ReadOptions` in
`crates/infra/src/db/read_options.rs`, whose fields are `datagrid_query` types:

```rust
pub struct ReadOptions {
    pub columns: Vec<SelectColumn>,
    pub filters: Vec<TypedFilter>,
    pub sort: Vec<Sort>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}
```

`TypedFilter` is the internal half of a filter: a condition plus its column's
`SqlType`. A generated read never asks a caller for that type, so a column name
cannot be typed by hand; the entity's fields lower to it inside the module.

`to_value` is the free-function form of `BindValue`, for a caller that wants
the converted value itself rather than a filter.

## Planned parameters

`ReadOptions::plan` builds the statement and pairs every parameter with the
column type of the filter that produced it:

```rust
pub struct PlannedQuery {
    pub sql: String,
    pub params: Vec<Param>,
}

pub struct Param {
    pub sql_type: SqlType,
    pub value: Value,
}
```

The pairing mirrors the builder's own parameter order: one value per
comparison, one per `in` element, one array for `any`, two for `between`, and
none for a null test or an empty `in` list. A count that disagrees is
`ReadError::Plan`, so a driver never guesses a type.

`SqlType` names the Postgres type a column binds as: `Bool`, `I16`, `I32`,
`I64`, `F32`, `F64`, `Decimal`, `Text`, `Uuid`, `Timestamptz`, `Timestamp`,
`Date`, `Time`, `Json`, and `Bytea`. The generator maps the spec's column type
to it, so a new column needs no generator edit.

## The executor port

`SqlExecutor` is the one database dependency of a generated read:

```rust
async fn query(&self, sql: &str, params: &[Param]) -> Result<Vec<Row>, ReadError>;
```

A driver implements it; the generated read builds the statement and hands over
its `sql` and `params`. `Row` is a JSON object keyed by the selected column
names, which is why the selected shape carries `#[serde(rename_all =
"camelCase")]`: a `select` of `WidgetSelectField::CreatedAt` returns the key
`createdAt`, and the decoder matches it.

## The sqlx driver

`SqlExecutor` is implemented for `sqlx::PgPool`, so a read runs on the pool
directly:

```rust
let widgets = query_widget(&pool, options).await?;
```

Binding is type-directed. sqlx declares each parameter's type when it prepares
the statement, so a `uuid` parameter must bind a `uuid`; the driver converts a
`Value` to the Rust type `Param::sql_type` names, and reports
`ReadError::InvalidValue` when it cannot. An `any` filter binds the array as one
parameter, one `Option` element per input so a null inside the list survives.

Rows come back as `PgRow`. The driver inspects each column's Postgres type and
decodes it to JSON: integers and floats as numbers, `numeric` as a string (a
decimal does not fit an `f64` faithfully), `uuid` and the date and time types as
their text forms, `json`/`jsonb` as the value itself, and `bytea` as bytes. A
column whose type is not one of those is `ReadError::Unsupported` rather than a
guess.

## Errors

`ReadError` is the read's one failure type, with a variant per stage: `Build`
(the options do not form a statement), `Plan` (the parameters and the filters'
types disagree), `Encode` (a model value has no JSON form), `Execute` (the
driver rejected the statement), `InvalidValue` (a value does not fit its column
type), `Unsupported` (a returned column has a type the decoder does not
handle), and `Decode` (a row does not match the selected shape).

## Regeneration

The generated output is committed. `pnpm generate:queries-rs` rewrites it and
removes a `query_<entity>.rs` whose entity no longer declares `@queries query`.
`run` in the generator is also part of `scripts/generate.ts`, so a full
`pnpm generate` refreshes the Rust queries with the other artifacts.

The hand-written `read_options.rs`, `sql_type.rs`, `executor.rs`, and
`postgres.rs` live beside the generated `queries/` directory, so regeneration
never writes over them.
