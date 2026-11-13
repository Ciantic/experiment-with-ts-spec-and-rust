# Rust queries

`scripts/infra/generate-queries-rs.ts` writes one read module per entity into
`crates/infra/src/db/queries/`. An entity with `@queries query` at
`spec/src/widget-service/Widget.ts` gets
`crates/infra/src/db/queries/query_widget.rs`. The crate writes no SQL of its
own: every read is built by `datagrid_query`. Rust and the TypeScript side share
the query's wire format, so one JSON body describes a read to either
backend, though each lowers it to its own SQL.

## The generated module

```rust
pub type WidgetQuery = EntityQuery<Widget>;

impl ReadEntity for Widget {
    type Select = WidgetSelect;
    type Selection = WidgetSelection;
    type Filter = WidgetFilter;
    type Order = WidgetOrder;

    const TABLE: &'static str = "widget";

    fn group(kind: FilterKind, filters: Vec<Self::Filter>) -> Self::Filter { … }

    fn lower(…) -> Result<LoweredRead, ReadError> { … }
}
```

The read takes the entity's own query type, `WidgetQuery`, so its selection and
ordering can only name `Widget`'s columns. The builder itself is written once, as
`EntityQuery<E>` in `crates/infra/src/db/entity_query.rs`, and a module declares
only what makes its entity different: the typed sets it is built from, the table
it plans against, how its filters group, and how they lower. The per-column
filter types and the entity's filter enum come from `flat_query_filters!`, whose
definition lives in `crates/macro-helpers`, so a column states its name, value
type, `SqlType`, and operators once and the macro writes every type over them.
`query` runs the statement and decodes every row into the model's selected
shape, `WidgetSelect`, from `crates/models` (`docs/rust-models.md`), and
`as_sql` reports the same statement as a plan — its SQL and typed parameters —
without the database, to log, assert on, or bind elsewhere. `as_sql` borrows,
so a read can be reported and then still run.

A field the read did not select stays `Partial::Unset`, so the caller can tell
"not read" from "null".

## Using a read

```rust
use infra::db::queries::{
    InvoiceDeletedAtFilter, InvoiceOrderField, InvoiceQuery, InvoiceSelectField, InvoiceTenantIdFilter,
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
    InvoiceQuery::with_select([
        InvoiceSelectField::Id,
        InvoiceSelectField::Number,
        InvoiceSelectField::TotalAmount,
        InvoiceSelectField::CreatedAt,
    ])
    .filter(InvoiceTenantIdFilter::Eq(tenant.clone()))
    .filter(InvoiceDeletedAtFilter::Eq(now))
    .order(InvoiceOrderField::CreatedAt.desc())
    .limit(limit)
    .query(pool)
    .await
}
```

`query` is the builder's last step, and it takes any `SqlExecutor`; `sqlx::PgPool`
implements it, so the pool passes straight through. The read then exists in one
expression, with no binding to name. Each row arrives as `InvoiceSelect`, where a
field the read did not select stays `Partial::Unset` — the caller can tell "not
read" from "null" without a second query.

Because `filter` takes `impl Into<InvoiceFilter>`, a column's own filter type can
be passed straight in, and the same column may be filtered more than once:

```rust
let invoice_query = InvoiceQuery::default()
    .filter(InvoiceTenantIdFilter::Eq(tenant.clone()))
    .filter(InvoiceTenantIdFilter::Any(other_tenants.clone()))
    .filter_or([
        InvoiceTenantIdFilter::Eq(tenant.clone()),
        InvoiceTenantIdFilter::Eq(other_tenant),
    ])
    .limit(1);
```

A builder is only needed when the read is passed somewhere; otherwise the same
chain ends in `query(pool).await`.

```
SELECT * FROM "invoice"
  WHERE "tenantId" = $1 AND "tenantId" = ANY($2)
    AND ("tenantId" = $3 OR "tenantId" = $4)
```

`filter_or` takes an iterator whose items convert into `InvoiceFilter`, so a list
of one column's type needs no conversion. A list that mixes columns converts each
item, `InvoiceFilter::from(filter)`, because a group holds one enum for every
column:

```rust
.filter_or([
    InvoiceFilter::from(InvoiceDeletedAtFilter::Eq(now)),
    InvoiceFilter::from(InvoiceTenantIdFilter::Neq(tenant.clone())),
])
```

One row is the first result of a limited read, since there is no separate `get`:

```rust
let invoice = invoice_query.query(pool).await?.pop();
```

An empty `select` reads every column, and `as_sql` returns the plan instead of
running it, so the read can be logged, asserted on, or bound before it is sent:

```rust
println!("{}", invoice_query.as_sql()?.sql);
// SELECT "id", "number", "totalAmount", "createdAt" FROM "invoice"
//   WHERE "tenantId" = $1 AND "deletedAt" IS NULL
//   ORDER BY "createdAt" DESC LIMIT 25
```

## Typed fields

Each module names the entity's columns once, as one type per capability:

| Type | Built from | Shape |
| --- | --- | --- |
| `WidgetSelectField` | every column | one variant per selectable column |
| `WidgetOrderField` | `@queryOrderBy` | one variant per orderable column, with `WidgetOrder` as the `[column, direction]` pair it builds |
| `Widget<Title>Filter` | one column | one variant per operator its `@queryFilter ops=` names |
| `WidgetFilter` | `@queryFilter` | one variant per filter column, plus the `and`/`or` group |

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
collects every column's type, plus the group:

```rust
#[serde(untagged)]
pub enum WidgetFilter {
    Id(WidgetIdFilter),
    TenantId(WidgetTenantIdFilter),
    DeletedAt(WidgetDeletedAtFilter),
    Group {
        kind: FilterKind,
        filters: Vec<Self>,
    },
}
```

`flat_query_filters!` writes it, its `From` impls, and its lowering from the
same column table, so a new column reaches the enum by being listed once. Every
column type converts into it (`impl From<WidgetTenantIdFilter> for WidgetFilter`),
so `filter` takes whichever the caller has. The group is a
struct variant rather than a second list of filters, so it travels on the wire as
`{ "kind": "and" | "or", "filters": [...] }` and cannot be confused with a
comparison. Its branches are the same enum, so a group nests to any depth and
`kind` decides how each level combines.

Because the read's `filters` is a list combined with `and`, the same column may
be filtered more than once:

```rust
WidgetQuery::default()
    .filter(WidgetTenantIdFilter::Eq(tenant.clone()))
    .filter(WidgetTenantIdFilter::Any(other_tenants))
// "tenantId" = $1 AND "tenantId" = ANY($2)
```

`filter_or` adds one group whose branches may match any, and `filter_and` one
whose branches must all match:

```rust
.filter_or([
    WidgetTenantIdFilter::Eq(a),
    WidgetTenantIdFilter::Eq(b),
])
// ("tenantId" = $1 OR "tenantId" = $2)
```

A branch is the same enum, so a group is a branch and the tree nests. `A and
(D or (E and F))` needs no rewriting:

```rust
WidgetQuery::default()
    .filter(WidgetTenantIdFilter::Eq(tenant))
    .filter_or([
        WidgetFilter::from(WidgetAccountIdFilter::Eq(first)),
        WidgetFilter::Group {
            kind: FilterKind::And,
            filters: vec![
                WidgetStartsAtFilter::Gte(from).into(),
                WidgetStartsAtFilter::Lte(to).into(),
            ],
        },
    ])
// "tenantId" = $1 AND ("accountId" = $2 OR ("startsAt" >= $3 AND "startsAt" <= $4))
```

The values keep the order they were given, so a group's parameters follow its
branches. The read's own list is still combined with `and`, which is why every
group stays parenthesised; a group of one branch lowers to that branch, and an
empty group constrains nothing.

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
reads as an equality filter. Each column is one line of the module's
`flat_query_filters!` invocation:

```rust
flat_query_filters! {
    Widget {
        tenantId: TenantId, Uuid, [eq],
        issueDate: chrono::DateTime<chrono::Utc>, Timestamptz, [gte, lte, between],
    }
}
```

The macro writes `WidgetTenantIdFilter` with its single `Eq` variant,
`WidgetIssueDateFilter` with `Gte`, `Lte`, and `Between`, and for each the flat
`column__operator` wire form and the typed filter the planner binds. Its
operators are the only place the wire spelling, the planner's spelling, and the
payload each takes are decided together.

The same invocation writes the entity's own `WidgetFilter`: one untagged variant
per column, the `Group` variant a read nests, a `From` impl per column type so
`filter` takes one directly, and `into_typed_filters`, which lowers a column to
its one entry and a group to its `and` or `or` combinator. A group of one branch
lowers to that branch and an empty group constrains nothing, so the tree the
caller built is the tree the statement carries.

## The query type

Each entity's query type is `EntityQuery` over its model, whose fields are the
entity's own typed sets:

```rust
pub struct EntityQuery<E: ReadEntity> {
    pub select: E::Selection,
    pub filters: Vec<E::Filter>,
    pub order: Vec<E::Order>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}
```

An empty `select` selects every column; `WidgetSelection` is a list of
`WidgetSelectField` that serializes as an object naming each one. A filter names
one column and one comparison, or a group of entries. A `WidgetOrder` pairs a
`WidgetOrderField` with a direction, and leaves nulls placement to the column's
own default for that direction. Both types only mention the entity's own columns,
so a query cannot name a column the spec forbids.

A query is built either literally or fluently:

```rust
let query = WidgetQuery::with_select([WidgetSelectField::Id, WidgetSelectField::Notes])
    .filter(WidgetTenantIdFilter::Eq(tenant_id))
    .order(WidgetOrderField::CreatedAt.desc())
    .limit(50);
```

Every builder takes and returns `self`, and the last step is either `query`, which
runs the read, or `as_sql`, which reports the plan it would run.

The entity's `ReadEntity::lower` converts the query to the source-independent
`ReadOptions` in `crates/infra/src/db/read_options.rs`, which arrives as the
opaque `LoweredRead` the planner consumes. Both are crate-internal; the fields
are `datagrid_query` types:

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

`WidgetQuery` and everything it holds implement `serde::Serialize` and
`serde::Deserialize`, so a read can cross a boundary — an HTTP body, a cache
entry, a test fixture — and come back:

```rust
let json = serde_json::to_string(&query)?;
let back: WidgetQuery = serde_json::from_str(&json)?;
assert_eq!(back, query);
```

The shape is the one the TypeScript read sends: a select is an object naming its
columns, a filter is one flat `column__operator` entry, and an ordering clause is
a `[column, direction]` pair.

```json
{
  "select": { "id": true, "number": true, "totalAmount": true, "createdAt": true },
  "filters": [
    { "tenantId__eq": "…" },
    { "deletedAt__isNull": true },
    { "id__any": ["…", "…"] },
    { "issueDate__gte": "2026-01-01T00:00:00Z" },
    { "issueDate__lte": "2026-02-01T00:00:00Z" },
    { "kind": "or", "filters": [
        { "deletedAt__isNull": true },
        { "kind": "and", "filters": [
            { "sellerId__isNotNull": true },
            { "id__any": ["…", "…"] }
        ] }
    ] }
  ],
  "order": [["createdAt", "desc"]],
  "limit": 25,
  "offset": 50
}
```

Each entry names one column and one operator, and the entries are combined with
`and`, so a column filtered twice is two entries. A group is the only entry
shaped differently: `kind` says whether all of its `filters` must match or any
may, and each of them is again an entry, so a group nests to any depth.

Decoding rejects everything the entity's own types do not allow:

- An unknown key in the query is an error.
- A column the entity does not declare is an error, and so is a column that
  carries no `@queryFilter` in `filters` or no `@queryOrderBy` in `order`.
- An operator the column's `ops=` does not name is an error, so a payload cannot
  reach a comparison the spec does not allow.
- An entry that names no comparison, or more than one, is an error: a filter entry
  is one operator on one column, so `{}` and `{"id__eq": "…", "id__neq": "…"}` both
  fail.
- A selected column whose value is not `true` is an error, so a selection cannot
  be read as a patch, and a null test whose value is not `true` is an error, so a
  `false` can never be read as a test.
- A value of the wrong type is an error at decode: `{"tenantId__eq": true}` fails
  because a `TenantId` is not a boolean. Nothing is coerced and no separate
  validation pass is needed.

Every part of the shape is optional: `{}` reads every column of every row. An
empty collection and an omitted one mean the same thing, so a round-trip drops an
empty `filters` or `order` rather than writing it. A selection is always written,
even empty, because the TypeScript read always sends one: `{}` and
`"select": {}` mean the same, and only the second is a body both sides accept. A
selection also decodes in its columns' alphabetical order, since it travels as an
object. An ordering clause must name its direction; the planner has no default for
it.

## Planned parameters

`ReadOptions::plan` builds the statement and pairs every parameter with the
column type of the filter that produced it. Both the generated `as_sql` and
`query` reach it; `query` binds the parameters as the statement declares them,
and `as_sql` hands the same plan back to the caller.

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
let widgets = widget_query.query(&pool).await?;
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
(the query does not form a statement), `Plan` (the parameters and the filters'
types disagree), `Encode` (a model value has no JSON form), `Execute` (the
driver rejected the statement), `InvalidValue` (a value does not fit its column
type), `Unsupported` (a returned column has a type the decoder does not
handle), and `Decode` (a row does not match the selected shape).

## Regeneration

The generated output is committed. `pnpm generate:queries-rs` rewrites it and
removes a `query_<entity>.rs` whose entity no longer declares `@queries query`.
`run` in the generator is also part of `scripts/generate.ts`, so a full
`pnpm generate` refreshes the Rust queries with the other artifacts.

The hand-written `flat_filter.rs`, `read_options.rs`, `sql_type.rs`,
`executor.rs`, and `postgres.rs` live beside the generated `queries/` directory,
so regeneration never writes over them.
