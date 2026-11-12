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
use infra::db::queries::{
    InvoiceDeletedAtFilter, InvoiceOrderField, InvoiceReadOptions, InvoiceSelectField, InvoiceTenantIdFilter,
    query_invoice,
};
use infra::db::ReadError;
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
    .filter(InvoiceTenantIdFilter::Eq(tenant.clone()))
    .filter(InvoiceDeletedAtFilter::Eq(now))
    .order(InvoiceOrderField::CreatedAt.desc().nulls_last())
    .limit(limit);

    query_invoice(pool, options).await
}
```

`query_invoice` takes any `SqlExecutor`; `sqlx::PgPool` implements it, so the pool
passes straight through. Each row arrives as `InvoiceSelect`, where a field the
read did not select stays `Partial::Unset` — the caller can tell "not read" from
"null" without a second query.

Because `filter` takes `impl Into<InvoiceFilter>`, a column's own filter type can
be passed straight in, and the same column may be filtered more than once:

```rust
let options = InvoiceReadOptions::default()
    .filter(InvoiceTenantIdFilter::Eq(tenant.clone()))
    .filter(InvoiceTenantIdFilter::Any(other_tenants.clone()))
    .filter_or([
        InvoiceTenantIdFilter::Eq(tenant.clone()),
        InvoiceTenantIdFilter::Eq(other_tenant),
    ])
    .limit(1);
```

```
SELECT * FROM "invoice"
  WHERE "tenantId" = $1 AND "tenantId" = ANY($2)
    AND ("tenantId" = $3 OR "tenantId" = $4)
```

`filter_or` and `filter_and` take an iterator whose items convert into
`InvoiceFilter`, so a list of one column's type needs no conversion. A list that
mixes columns converts each item, `InvoiceFilter::from(filter)`:

```rust
.filter_or([
    InvoiceFilter::from(InvoiceDeletedAtFilter::Eq(now)),
    InvoiceFilter::from(InvoiceTenantIdFilter::Neq(tenant.clone())),
])
```

One row is the first result of a limited read, since there is no separate `get`:

```rust
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

## Typed fields

Each module names the entity's columns once, as one type per capability:

| Type | Built from | Shape |
| --- | --- | --- |
| `WidgetSelectField` | every column | one variant per selectable column |
| `WidgetOrderField` | `@queryOrderBy` | one variant per orderable column, with `WidgetOrder` as the clause it builds |
| `Widget<Title>Filter` | one column | one variant per operator its `@queryFilter ops=` names |
| `WidgetFilter` | `@queryFilter` | one variant per filter column, plus `and` and `or` groups |

A column an `@inlined` branch reaches through that branch is not selectable by
name, so it is absent from `WidgetSelectField`. An entity that declares no
ordering column gets an `WidgetOrderField` with no variants, so a caller cannot
name one. A column the spec does not mark `@queryFilter` has no filter type of its
own, so there is nothing to pass to `filter`.

The select and order enums carry `ALL`, `as_str`, and `Display`, plus the helper
their capability needs:

```rust
WidgetSelectField::Notes.select();                          // SelectColumn::Name("notes")
WidgetOrderField::CreatedAt.desc();                         // "createdAt" DESC
```

## Values

A filter is one comparison, on one column. Each filterable column has its own
type, so a value of the wrong type does not compile, and a column the spec does
not expose has no type to name:

```rust
pub enum WidgetTenantIdFilter {
    Eq(TenantId),
    Neq(TenantId),
}
```

The variants are the column's `@queryFilter ops=` and nothing else, and each one
carries the model type of that column, so a value of the wrong type does not
compile and a comparison the spec omits cannot be written. The umbrella enum
collects every column's type, plus the groups:

```rust
#[serde(rename_all = "camelCase")]
pub enum WidgetFilter {
    Id(WidgetIdFilter),
    TenantId(WidgetTenantIdFilter),
    DeletedAt(WidgetDeletedAtFilter),
    And(Vec<Self>),
    Or(Vec<Self>),
}
```

Every column type converts into it (`impl From<WidgetTenantIdFilter> for
WidgetFilter`), so `filter` takes whichever the caller has.

Because the read's `filters` is a list combined with `and`, the same column may
be filtered more than once:

```rust
WidgetReadOptions::default()
    .filter(WidgetTenantIdFilter::Eq(tenant.clone()))
    .filter(WidgetTenantIdFilter::Any(other_tenants))
// "tenantId" = $1 AND "tenantId" = ANY($2)
```

`filter_and` and `filter_or` take a set of filters:

```rust
.filter_or([
    WidgetTenantIdFilter::Eq(a),
    WidgetTenantIdFilter::Eq(b),
])
// ("tenantId" = $1 OR "tenantId" = $2)
```

An `And` group is flattened into the enclosing list, since a branch's children
are already combined with `and`; an `Or` group stays parenthesised.

Lowering a filter to the builder's statement happens once, inside the module:
`into_typed_filter` maps each set column to its column name and `SqlType` and
converts the typed value to a bind parameter. It is the only fallible step, and
only a value with no JSON form can fail it.

## Operators

A column's variant list is exactly its `@queryFilter ops=`, so a range is two
filters on one column and the operators no annotation names cannot be written:

```rust
WidgetIssueDateFilter::Gte(from)
WidgetIssueDateFilter::Lte(to)
// "issueDate" >= $1 AND "issueDate" <= $2
```

How many values a variant carries follows its operator, which is what makes the
wire shape unambiguous:

| Operators | Variant payload |
| --- | --- |
| `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `notLike`, `notIlike` | the column's value |
| `in`, `notIn`, `any`, `notAny` | `Vec` of the column's value |
| `between`, `notBetween` | two values |
| `isNull`, `isNotNull` | nothing |

```rust
WidgetDeletedAtFilter::IsNull                 // "deletedAt" IS NULL
WidgetCustomerIdFilter::Any(customer_ids)     // "customerId" = ANY($1)
WidgetIssueDateFilter::Between(from, to)      // "issueDate" BETWEEN $1 AND $2
```

A column with no `ops=` offers `eq` alone, which is why a bare `@queryFilter`
reads as an equality filter:

```rust
pub enum WidgetTenantIdFilter {
    Eq(TenantId),
}
```

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

An empty `select` selects every column. A filter names one column and nests
further filters in `and` or `or`. A `WidgetOrder` pairs a
`WidgetOrderField` with a direction and an optional nulls order. Both types only
mention the entity's own columns, so an options value cannot name a column the
spec forbids.

The options are built either literally or fluently:

```rust
let options = WidgetReadOptions::with_select([WidgetSelectField::Id, WidgetSelectField::Notes])
    .filter(WidgetTenantIdFilter::Eq(tenant_id))
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

A select field is its column name, a filter is one column with one comparison,
and the comparison is the operator:

```json
{
  "select": ["id", "number", "totalAmount", "createdAt"],
  "filters": [
    { "tenantId": { "eq": "…" } },
    { "deletedAt": "isNull" },
    { "id": { "any": ["…", "…"] } },
    { "issueDate": { "gte": "2026-01-01T00:00:00Z" } },
    { "issueDate": { "lte": "2026-02-01T00:00:00Z" } },
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

Each entry names one column, and the entries are combined with `and`, so a column
filtered twice is two entries.

Decoding rejects everything the entity's own types do not allow:

- An unknown key in the options or in an ordering clause is an error.
- A column the entity does not declare is an error, and so is a column that
  carries no `@queryFilter` in `filters` or no `@queryOrderBy` in `order`.
- An operator the column's `ops=` does not name is an error, so a payload cannot
  reach a comparison the spec does not allow.
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
