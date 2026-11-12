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

## Using a read

```rust
use infra::db::ReadError;
use infra::db::queries::{
    InvoiceFilter, InvoiceOrderField, InvoiceReadOptions, InvoiceSelectField, query_invoice,
};
use models::domain::invoicing_service::invoice::InvoiceSelect;
use models::domain::user_service::tenant::TenantId;
use sqlx::PgPool;

/// A page of a tenant's open invoices, newest first.
async fn open_invoices(
    pool: &PgPool,
    tenant: &TenantId,
    limit: u64,
) -> Result<Vec<InvoiceSelect>, ReadError> {
    let options = InvoiceReadOptions::with_select([
        InvoiceSelectField::Id,
        InvoiceSelectField::Number,
        InvoiceSelectField::TotalAmount,
        InvoiceSelectField::CreatedAt,
    ])
    .filter(InvoiceFilter::TENANT_ID.eq(tenant))
    .filter(InvoiceFilter::DELETED_AT.is_null())
    .order(InvoiceOrderField::CreatedAt.desc().nulls_last())
    .limit(limit);

    query_invoice(pool, options).await
}
```

`query_invoice` takes any `SqlExecutor`; `sqlx::PgPool` implements it, so the pool
passes straight through. Each row arrives as `InvoiceSelect`, where a field the
read did not select stays `Partial::Unset` — the caller can tell "not read" from
"null" without a second query.

One row is the first result of a limited read, since there is no separate `get`:

```rust
let options = InvoiceReadOptions::default()
    .filter(InvoiceFilter::TENANT_ID.eq(tenant))
    .filter(InvoiceFilter::ID.eq(id))
    .limit(1);

let invoice = query_invoice(pool, options).await?.pop();
```

An empty `select` reads every column, and `build_query_invoice` returns the
statement without running it:

```rust
let planned = build_query_invoice(options)?;
println!("{} {:?}", planned.sql, planned.params);
// SELECT "id", "number", "totalAmount", "createdAt" FROM "invoice"
//   WHERE "tenantId" = $1 AND "deletedAt" IS NULL
//   ORDER BY "createdAt" DESC NULLS LAST LIMIT 25
```

Filters combine with `and`; a nested group is an `InvoiceFilter`:

```rust
InvoiceReadOptions::default()
    .filter(InvoiceFilter::TENANT_ID.eq(tenant))
    .filter(InvoiceFilter::CUSTOMER_ID.any(customer_ids))
    .filter(InvoiceFilter::or([
        InvoiceFilter::SELLER_ID.is_not_null(),
        InvoiceFilter::DELETED_AT.is_not_null(),
    ]))
```

## Typed fields

Each module names the entity's columns once, as one type per capability:

| Type | Built from | Fields |
| --- | --- | --- |
| `WidgetSelectField` | every column | selectable |
| `WidgetFilter` | `@queryFilter` | filterable, one variant and one builder per column |
| `WidgetOrderField` | `@queryOrderBy` | orderable, with `WidgetOrder` as the clause it builds |

A column an `@inlined` branch reaches through that branch is not selectable by
name, so it is absent from `WidgetSelectField`. An entity that declares no
ordering column gets an `WidgetOrderField` with no variants, so a caller cannot
name one. A filter needs no second enum: the `WidgetFilter` variant already
names the column, so a column the spec does not mark `@queryFilter` has neither
a variant nor a builder.

The select and order enums carry `ALL`, `as_str`, and `Display`, plus the helper
their capability needs:

```rust
WidgetSelectField::Notes.select();                          // SelectColumn::Name("notes")
WidgetOrderField::CreatedAt.desc();                         // "createdAt" DESC
```

The filter builders are covered under "Values".

## Values

A filter is an enum whose variant names the column, and whose payload has that
column's model type:

```rust
#[serde(rename_all = "camelCase")]
pub enum WidgetFilter {
    Id(Comparison<WidgetId>),
    TenantId(Comparison<TenantId>),
    DeletedAt(Comparison<chrono::DateTime<chrono::Utc>>),
    And(Vec<Self>),
    Or(Vec<Self>),
}
```

`Comparison<T>` is the operator vocabulary over one value type:

```rust
pub enum Comparison<T> {
    Eq(T),
    Ne(T),
    Any(Vec<T>),
    IsNull,
    IsNotNull,
}
```

Because the type parameter is the column's own type, a payload cannot carry a
value the column does not accept: `{"tenantId": {"eq": true}}` fails to
deserialize, and the message names the expected type. Nothing is coerced and
nothing has to be checked later.

The builders are the ergonomic route to a variant. `WidgetFilter::TENANT_ID` is
the `TenantId` builder, so `eq` and `ne` take `impl Borrow<TenantId>` (a value or
a reference to one) and are infallible:

```rust
WidgetFilter::TENANT_ID.eq(&tenant_id)                           // { "tenantId": { "eq": "…" } }
WidgetFilter::ID.any([first_id, second_id])                      // { "id": { "any": ["…", "…"] } }
WidgetFilter::DELETED_AT.is_null()                               // { "deletedAt": "isNull" }
WidgetFilter::NOTES.eq("draft")                                  // a plain string column
```

A builder is a zero-sized value, so the constant is the builder: there is no
separate field type to name the column twice.

Passing a `CustomerId` to the `TenantId` builder does not compile, and the
builder cannot reach a column the spec does not mark `@queryFilter`.

`WidgetFilter::and(…)` and `WidgetFilter::or(…)` nest filters; each child keeps
its own column type:

```rust
WidgetFilter::or([
    WidgetFilter::NUMBER.is_null(),
    WidgetFilter::SUPPLIER_ID.is_not_null(),
])
```

Lowering a filter to the builder's statement happens once, inside the module:
`WidgetFilter::into_typed_filter` maps each variant to its column name and
`SqlType` and converts the typed value to a bind parameter. That conversion is
the only fallible step, and only a value with no JSON form can fail it, so
`ReadOptions` gains no validation of its own.

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

An empty `select` selects every column. A `WidgetFilter` names a column through
its variant and carries that column's comparison, either directly or inside an
`and` / `or` group. A `WidgetOrder` pairs a `WidgetOrderField` with a direction
and an optional nulls order. Both types only mention the entity's own columns,
so an options value cannot name a column the spec forbids.

The options are built either literally or fluently:

```rust
let options = WidgetReadOptions::with_select([WidgetSelectField::Id, WidgetSelectField::Notes])
    .filter(WidgetFilter::TENANT_ID.eq(&tenant_id))
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

## JSON

`WidgetReadOptions` and everything it holds implement `serde::Serialize` and
`serde::Deserialize`, so a read can cross a boundary — an HTTP body, a cache
entry, a test fixture — and come back:

```rust
let json = serde_json::to_string(&options)?;
let back: WidgetReadOptions = serde_json::from_str(&json)?;
assert_eq!(back, options);
```

A field enum is its column name, a filter variant is the column it targets, and
a comparison is the operator:

```json
{
  "select": ["id", "number", "totalAmount", "createdAt"],
  "filters": [
    { "tenantId": { "eq": "…" } },
    { "id": { "any": ["…", "…"] } },
    { "deletedAt": "isNull" },
    { "or": [
        { "deletedAt": "isNull" },
        { "sellerId": "isNotNull" }
    ] }
  ],
  "order": [{ "field": "createdAt", "direction": "desc", "nulls": "last" }],
  "limit": 25,
  "offset": 50
}
```

Decoding rejects everything the entity's own types do not allow:

- An unknown key in the options or in an ordering clause is an error.
- A column the entity does not declare is an error, and so is a column that
  carries no `@queryFilter` in `filters` or no `@queryOrderBy` in `order`.
- An operator the filter does not offer is an error, so a payload cannot reach a
  comparison the spec does not allow.
- A value of the wrong type is an error at decode: `{"tenantId": {"eq": true}}`
  fails because a `TenantId` is not a boolean. Nothing is coerced and no
  separate validation pass is needed.

Every part of the shape is optional: `{}` reads every column of every row. An
empty collection and an omitted one mean the same thing, so a round-trip drops
`"select": []` rather than writing it. An ordering clause may omit `direction`,
which reads as `asc`.

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
