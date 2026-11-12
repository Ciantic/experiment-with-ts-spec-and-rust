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
| `WidgetFilterField` | `@queryFilter` | filterable, with one typed builder per column |
| `WidgetOrderField` | `@queryOrderBy` | orderable, with `WidgetOrder` as the clause it builds |

A column an `@inlined` branch reaches through that branch is not selectable by
name, so it is absent from `WidgetSelectField`. An entity that declares no
filter or ordering column gets the matching enum with no variants, so a caller
cannot name one.

Every enum carries `ALL`, `as_str` and `Display`, plus the helpers its
capability needs:

```rust
WidgetSelectField::Notes.select();                          // SelectColumn::Name("notes")
WidgetFilterField::TENANT_ID.eq(&tenant_id)?;               // "tenantId" = $1, a TenantId
WidgetFilterField::ID.any([first_id, second_id])?;          // "id" = ANY($1), one array parameter
WidgetFilterField::DELETED_AT.is_null();                    // "deletedAt" is null
WidgetOrderField::CreatedAt.desc();                         // "createdAt" DESC
```

A filter field is reached through a constant named for its column, and that
constant returns a builder of the column's own value type. `sql_type()` maps
the enum variant to the type the driver binds. `WidgetFilter::and(…)` and
`WidgetFilter::or(…)` nest filters, and each child keeps its own column type.

```rust
WidgetFilter::or([
    WidgetFilterField::NUMBER.is_null(),
    WidgetFilterField::SUPPLIER_ID.is_not_null(),
])
```

## Values

Each filter builder takes that column's model type, so a value of the wrong
type is a compile error rather than a query that fails at run time:

```rust
pub struct WidgetTenantIdFilter(WidgetFilterField);

impl WidgetTenantIdFilter {
    pub fn eq(self, value: impl Borrow<TenantId>) -> Result<WidgetFilter, ReadError>;
    pub fn ne(self, value: impl Borrow<TenantId>) -> Result<WidgetFilter, ReadError>;
    pub fn any(self, values: impl IntoIterator<Item = impl Borrow<TenantId>>) -> Result<WidgetFilter, ReadError>;
    pub fn is_null(self) -> WidgetFilter;
    pub fn is_not_null(self) -> WidgetFilter;
}
```

`Borrow<T>` accepts the value and a reference to it, so a caller keeps its own
copy without cloning:

```rust
WidgetFilterField::TENANT_ID.eq(&tenant_id)?;                // a borrow of a branded id
WidgetFilterField::AMOUNT.eq(amount)?;                       // a Money
WidgetFilterField::ID.any([first_id, second_id])?;           // ids, bound as one array
WidgetFilterField::NOTES.eq("draft")?;                       // a plain string column
```

The value type comes from the same mapping that writes the models crate
(`docs/rust-models.md`), so a branded id arrives as its own newtype, a `Date`
column as `chrono::DateTime<Utc>`, and a `string` column as `String`. Passing a
`CustomerId` to a `TenantId` filter does not compile.

A value becomes a bind value through the crate's `BindValue`, which any
`Serialize` type satisfies, so a newtype binds the value it wraps. The
conversion can only fail when a value has no JSON form, which is why the
value-carrying methods return `Result<WidgetFilter, ReadError>`; the null tests
bind nothing and return a `WidgetFilter` directly.

The raw `datagrid_query::Value` never reaches a caller. The filter enum's
`condition` constructor and the `ReadOptions` lowering are crate-internal, and
`BindValue` is a crate-internal trait, so the typed builders are the only way
in: a filter's value is always the column's model type.

The field enums are the validity check: a column the spec does not expose
cannot be named, so a caller never builds a filter or an ordering the entity
forbids. A raw column name has no route into a generated read.

## Read options

Each generated module declares the entity's read options, whose fields are the
entity's own typed sets:

```rust
pub struct WidgetReadOptions {
    pub select: Vec<WidgetSelectField>,
    pub filters: Vec<WidgetFilter>,
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
    .filter(WidgetFilterField::TENANT_ID.eq(&tenant_id)?)
    .order(WidgetOrderField::CreatedAt.desc().nulls_last())
    .limit(50);
```

`into_read_options` lowers them to the source-independent `ReadOptions` in
`crates/infra/src/db/read_options.rs`. Both that method and the type are
crate-internal; the fields are `datagrid_query` types:

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
`SqlType`. It, `ReadOptions`, and `BindValue` are crate-visible only, so a
column name cannot be typed by hand and a raw bind value cannot be supplied;
the entity's fields lower to them inside the module.

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
