# Queries

Reading is derived from the spec. An entity that carries `@queries query` in
`packages/spec/src/` gets a generated `query<Entity>` read; a caller that
wants a single row takes the first result. A generator emits the typed functions
and the physical model; one hand-written resolver turns a selection into SQL.
See "Why the resolver is hand-written".

- `packages/sdk/src/selection.ts` — `Selection`/`Selected` (the
  column-limiting types), `Filters`/`Filter` (the filter arguments), and the
  `Order` clause. Hand-written; the resolver and the generated REST client
  import their types from it, so the read contract has one definition. See
  `docs/rest-api.md`.
- `packages/sdk/src/filters.ts` — generated: one `<Entity>Filter` operator map per
  entity that declares a read, so the read, its guard, and its client name one
  definition. `scripts/sdk/generate-filters.ts` writes it.
- `scripts/infra/generate-queries.ts` — the generator.
- `packages/infra/src/db/queries/` — generated: `model.ts` (metadata),
  `query<Entity>.ts` (a `query<Entity>` per `@queries` entity),
  `watch<Entity>.ts` (a change stream per entity that declares a scope),
  `get<Entity>Tenants.ts` (the tenants the named rows belong to, for a
  `@tenantOwned` entity only), and `index.ts` (barrel). See
  `docs/notifications.md` and `docs/permissions.md`.
- `packages/infra/src/db/resolvers.ts` — the reader. Hand-written.
- `packages/sdk/src/queries/` — generated read argument
  schemas. See `docs/validation.md`.

- `pnpm generate:queries` — writes the generated modules.
- `pnpm generate:queries --out <dir>` — writes elsewhere. A missing directory is created.

The generated output is committed. Regenerate rather than editing it by hand. A
regeneration also removes a module that no entity declares, so an entity that
drops `@queries query` leaves no stale file behind.

## One query per entity

An entity declares its read with `@queries <operation>…`; `query` is the only
read operation, so the tag is written `@queries query`. An entity that declares
no read gets no `query<Entity>` and no read route — the generator writes a
module only for the tables the tag names. Change streams are a separate
declaration: `@watchAll` and `@watchRows` declare the `all` and `rows` scopes,
`@restWatch` exposes a subset, and `@watchAllFilter` / `@watchRowsFilter` mark
the fields each scope may filter on. See
`docs/spec-annotations.md` and `docs/notifications.md`.

There is no query alias to write. The generator walks the tables and emits
`query<Entity>`:

```ts
export function queryWidget<S extends Selection<Widget>>(
    db: SqlExecutor,
    opts: {
        filters?: Filters<WidgetFilter>;
        order?: Order<"createdAt" | "updatedAt">[];
        limit?: number;
        offset?: number;
        select: S;
    },
): Promise<Selected<Widget, S>[]> {
    return resolver.resolveMany<Widget, S>(db, "widget", {
        select: opts.select,
        filters: opts.filters,
        order: opts.order,
        limit: opts.limit,
        offset: opts.offset,
    });
}
```

No domain name is hardcoded: a new entity flows through with no generator edit.
An entity named `Account` gets `queryAccount`, and `WidgetRow` gets
`queryWidgetRow`, the same way.

`model.ts` names every table, whether or not the entity reads: a read may select
into a table that has no read of its own, so the resolver needs its key, fields,
and branches. Dropping a table from the model would break the read that navigates
into it.

To read one row, take the first result — there is no `get`. A filter set that
matches several rows yields them in the order the read names (or the entity
default), at most `limit` of them, in pages of `offset`.

## `@queryFilter`

```
@queryFilter
@queryFilter ops=gte,lte
@queryFilter default=<value>
```

A marker on a **scalar** field. It adds the field as a filter of the entity's
reads. `id` is a filter **without the tag**, because each entity has one, so
`queryWidgetRow` accepts a filter on `id` out of the box:

```ts
const rows = await queryWidgetRow(db, {
    filters: [{ tenantId__eq: tenantId }, { id__any: [firstId, secondId] }],
    select: { description: true, subtotal: true },
});

const [row] = await queryWidgetRow(db, {
    filters: [{ tenantId__eq: tenantId }, { id__eq: rowId }],
    select: { description: true, subtotal: true },
});
```

The tag is what adds a filter *besides* `id`. `Widget.code` would become
filterable by marking it:

```ts
/**
 * @fieldName Widget code
 * @unique
 * @widget text
 * @queryFilter
 */
code?: string;
```

The default lives in `spec-model.ts`, which sets the tag when it parses a field
carrying `@primaryKey`; a generator reads the tag and never the name. Writing the
tag on a `@primaryKey` field adds nothing to that default, so it is a lint
finding. A composite key contributes one filter per key column, so each part of
the key filters on its own.

A field without `@queryFilter` is not filterable. Branch fields
(`@relation`/`@children`/`@inlined`) may not carry it; a relation is filtered
through its `@foreignKey` field, which is a scalar like any other.

### A filter entry is one flat comparison

A read carries `filters`: a list of entries in which each entry holds **one**
`column__operator` key. The entries are combined with **`and`**, so one column may
appear more than once — which is how a range, or a match on either of two sets,
is written:

```ts
const widgets = await queryWidget(db, {
    filters: [
        { tenantId__eq: tenantId },
        { startsAt__gte: from },
        { startsAt__lte: to },
    ],
    select: { code: true },
});
// "tenantId" = $1 AND "startsAt" >= $2 AND "startsAt" <= $3
```

The generated type is the whole union, so each entry is one key with that
column's value type, and an operator its `ops=` does not name is not a member at
all. A read, its route guard, and its client all name the same type:

```ts
export type FilterGroup<F> = {
    kind: "and" | "or";
    filters: readonly FilterEntry<F>[];
};

export type FilterEntry<F> = F | FilterGroup<F>;

export type Filters<F> = readonly FilterEntry<F>[];

/** One comparison a filter over `Widget` may name, as a flat `column__operator` entry. */
export type WidgetFilter =
    | { id__eq: WidgetId }
    | { tenantId__eq: TenantId }
    | { accountId__eq: AccountId }
    | { accountId__any: readonly AccountId[] }
    | { startsAt__gte: Date }
    | { startsAt__lte: Date }
    | { startsAt__between: readonly [Date, Date] }
    | { deletedAt__isNull: true };
```

An entry that names a column the entity does not mark, or an operator that column
does not offer, is a **type error** and, over HTTP, a 400. An entry with two keys
is one too, because the value is a union of single-key objects. Left out,
`filters` is an empty list, so `queryTenant(db, { select })` lists every row.

### A group

Every entry is `and`ed, and an entry written as a group says how its own branches
combine — `or` when any may match:

```ts
const widgets = await queryWidget(db, {
    filters: [
        { tenantId__eq: tenantId },
        { deletedAt__isNull: true },
        {
            kind: "or",
            filters: [{ accountId__eq: first }, { accountId__eq: second }],
        },
    ],
    select: { code: true },
});
// "tenantId" = $1 AND "deletedAt" IS NULL AND ("accountId" = $2 OR "accountId" = $3)
```

A branch is an entry, so it is a comparison or another group:

```ts
filters: [{ kind: "or", filters: [{ startsAt__gte: from }, { startsAt__isNull: true }] }]
// ("startsAt" >= $1 OR "startsAt" IS NULL)
```

Because a branch is an entry, a group nests to any depth, so `A and B and
(D or (E and F))` needs no rewriting:

```ts
const widgets = await queryWidget(db, {
    filters: [
        { tenantId__eq: tenantId },
        { deletedAt__isNull: true },
        {
            kind: "or",
            filters: [
                { accountId__eq: first },
                {
                    kind: "and",
                    filters: [{ startsAt__gte: from }, { startsAt__lte: to }],
                },
            ],
        },
    ],
    select: { code: true },
});
// "tenantId" = $1 AND "deletedAt" IS NULL
//   AND ("accountId" = $2 OR ("startsAt" >= $3 AND "startsAt" <= $4))
```

An `and` group says nothing a root list does not, so it earns its place only as a
branch of an `or`; a group of one branch is that branch, and an empty group
constrains nothing, exactly as an empty list does. A group's entries are checked
against the same generated type as a root entry, so an unknown column or operator
fails at any depth.

A nullable column's null test is a variant like any other, which is how a read
names the rows a default would hide:

```ts
// Live widgets: the default fills in `isNull`.
const live = await queryWidget(db, { select: { code: true } });

// Deleted widgets: the null test replaces the default.
const deleted = await queryWidget(db, {
    filters: [{ deletedAt__isNotNull: true }],
    select: { code: true, deletedAt: true },
});
```

`__isNull` and `__isNotNull` take `true` and nothing else, so the entry still
carries exactly one value. They are offered only for an optional field, whose
column can hold null; a null test on a not-null column would match every row or
none, so the linter rejects an `ops=` that names either there.

A filter whose column defaults is applied unless an entry names it, so the
default only fills a hole.

### A defaulted filter

`@queryFilter default=<value>` gives the filter a value to match when a read
omits it. The value is one of the null-test keywords `isNull` and `isNotNull`, a
boolean, a number, or a string. A bare token is a string, so a date or an enum
value needs no quotes (`default=2024-01-01`, `default=draft`), and quotes are
also accepted (`default="draft"`). A quoted keyword is a string too, so a text
column can default to the text `isNull`.

```ts
/**
 * @fieldName Deleted at
 * @deletedAt
 * @queryFilter default=isNull
 * @widget date
 */
deletedAt?: Date;
```

A read that leaves `deletedAt` out matches only the rows where the column is
`null`, because `isNull` compiles to `is null`. That is how soft deletion is
modelled: the column is an ordinary nullable field, `update` sets the moment,
and every read hides trashed rows unless it names the filter. The `@deletedAt`
marker adds no behaviour of its own; it only makes the linter check that shape.
See `docs/soft-delete.md`.

A default is one comparison: `=` against a literal, or a null test. An entry that
names the column overrides it, so the default only fills a hole. Defaults combine
with the other entries with `and`.

- **A null default needs a nullable field.** `default=isNull` and
  `default=isNotNull` read a nullable column; on a not-null one the first would
  never match and the second would match everything, so the linter reports both.
  A literal works on either.
- **A default applies to branches too.** A branch narrows through the same
  resolver, so a `@children` or `@relation` branch over a defaulted entity hides
  the defaulted rows unless its own arguments name the filter.

## `@queryOrderBy`

```
@queryOrderBy
@queryOrderBy default asc
```

A bare marker on a **scalar** field whitelists it as an ordering key. The
`order` option then accepts clauses that name those fields:

```ts
const widgets = await queryWidget(db, {
    filters: [{ tenantId__eq: tenantId }, { accountId__eq: accountId }],
    order: [["createdAt", "desc"]],
    select: { code: true },
});
```

`order` is an array of `[field, direction]` tuples, so several clauses sort with
the first breaking ties, each column matching SQL `ORDER BY`. Only whitelisted
fields may order — an unknown field is a **type error** and, over HTTP, a 400 —
so a field simply cannot reach the `ORDER BY`.

```ts
/** The ordering of a read: `[field, direction]` per sort key. See docs/queries.md. */
export type Order<K extends PropertyKey> = [field: K, direction: Direction];
export type Direction = "asc" | "desc";
```

### The default ordering

`@queryOrderBy default asc|desc` (at most one field per entity) additionally
makes that field the **entity default**: a read that names no `order` sorts by
it.

```ts
/**
 * @fieldName Created at
 * @queryOrderBy default asc
 * @widget date
 */
createdAt?: Date;

/**
 * @fieldName Updated at
 * @queryOrderBy
 * @widget date
 */
updatedAt?: Date;
```

An entity with no `@queryOrderBy` field takes no `order` (its opts type omits
the key) and is returned in database order. A branch carries its own `order`
inside its read arguments, the same way it carries a `filter`; the target's
whitelist is what an order clause is checked against, so a branch orders only by
its own entity's fields.

## `@queryFilter ops=`

```
@queryFilter ops=eq,neq,gt,gte,lt,lte
```

`ops=` whitelists the comparisons a filterable column may be compared with. Left
out, the column offers `eq` alone, so a bare `@queryFilter` reads as an equality
filter.

```ts
/**
 * @fieldName Starts at
 * @queryFilter ops=gte,lte,between
 * @widget date
 */
startsAt?: Date;
```

```ts
const widgets = await queryWidget(db, {
    filters: [{ startsAt__gte: from }, { startsAt__lte: to }],
    select: { code: true },
});

// The same range as one entry.
const ranged = await queryWidget(db, {
    filters: [{ startsAt__between: [from, to] }],
    select: { code: true },
});
```

The vocabulary is the query builder's own, spelled as it spells them, and each
operator decides the value it takes:

| Operators | Value |
| --- | --- |
| `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `notLike`, `notIlike` | one value of the column's type |
| `in`, `notIn`, `any`, `notAny` | a list of it |
| `between`, `notBetween` | exactly two of it |
| `isNull`, `isNotNull` | `true`, which binds no parameter |

`any` is the index-friendly membership test — `"tenantId" = ANY($1)` — and `in`
is the `in (…)` list. An operator the column's `ops=` does not name is a **type
error** and, over HTTP, a 400; `ops=` names at least one, so an empty `ops=` is a
lint finding. The order of the tokens does not matter, because the parser
canonicalizes them.

## `limit` and `offset`

Every read pages. `limit` bounds how many rows a read returns and `offset` skips
leading rows.

```ts
const page = await queryWidget(db, {
    filters: [{ tenantId__eq: tenantId }, { accountId__eq: accountId }],
    order: [["createdAt", "desc"]],
    limit: 25,
    offset: 50,
    select: { code: true },
});
```

`limit` defaults to **1000** and `offset` to **0**, so a root read that names
neither returns the first 1000 matching rows. The default is the resolver's
`DEFAULT_LIMIT`; pass a larger `limit` to widen the window, or a smaller one to
want less.

A branch names its own `limit` and `offset` in its read arguments, and there they
bound **each parent**, not the batch: `rows: { select: {…}, limit: 1 }` returns the
first row of each widget, not the first row of all widgets. A branch that names
neither is unbounded. The resolver ranks a branch's rows per parent
(`row_number()` over the parent key) and keeps each parent's window, so a batched
branch still costs one query.

A non-positive `limit` or a negative `offset` is a **type error** and, over
HTTP, a 400; the resolver re-checks both, because a decoded number reaches the
SQL unchecked. There is no cursor pagination and no `limit: null` escape hatch:
`limit` is always a positive integer, the default cap included.

## Selection

A query does not return a whole entity by default. It returns the fields the
caller asks for, narrowed in the type system. `Selection` describes what may be
asked; `Selected` is the result.

```ts
/** The scalar field names of `E`: the fields a branch's `filters` or `order` may name. */
export type ScalarKeys<E> = {
    [K in keyof E]-?: Scalar<E[K]> extends true ? K : never;
}[keyof E];

/** The arguments of a read over `E`, as a branch carries them: the same shape a root read takes. */
export type QueryArgs<E> = {
    select: Selection<E>;
    filters?: Filters<Filter<E, Partial<Record<Extract<ScalarKeys<E>, string>, CompareOp>>>>;
    order?: readonly (readonly [field: ScalarKeys<E>, direction: Direction])[];
    limit?: number;
    offset?: number;
};

/** The fields a caller wants: `true` for a scalar, `true` or a read's arguments for a branch. */
export type Selection<E> = {
    [K in keyof E]?: Scalar<E[K]> extends true ? true : true | QueryArgs<ElementOf<E[K]>>;
};

/** The value one selected key carries: the entity's field, or the selected shape of a branch. */
type SelectedValue<E, S, K extends keyof E & keyof S> = NonNullable<S[K]> extends true
    ? E[K]
    : IsArray<E[K]> extends true
        ? Selected<ElementOf<E[K]>, BranchSelect<S[K]>>[]
        : Selected<ElementOf<E[K]>, BranchSelect<S[K]>>;

/** The result of `E` under selection `S`: only the selected keys, each keeping the entity's optionality. */
export type Selected<E, S> = {
    [K in keyof E as K extends keyof S ? K : never]: SelectedValue<E, S, K & keyof S>;
};
```

A scalar is selected with `true`; a branch (a child collection or a relation) is
selected either with `true` — every scalar of the branch, unfiltered — or with a
**read's arguments**: a required `select`, plus the `filters`, `order`, `limit`,
and `offset` a root read takes. Properties not named are absent from the
result, not `undefined`.

```ts
const [widget] = await queryWidget(db, {
    filters: [{ tenantId__eq: tenantId }],
    select: {
        code: true,
        // The same arguments `queryWidgetRow` takes, applied to this widget's rows.
        rows: {
            select: { description: true, subtotal: true },
            order: [["subtotal", "desc"]],
            limit: 3,
        },
    },
});
```

A branch's `select` is mandatory, so the empty object is not a branch. The
runtime reads the same annotations a root read does: a branch's `filters`/`order`
is checked against the **target's** whitelists, so an unannotated field is a
runtime throw (and a 400 over HTTP) even though the branch's `filters` and
`order` are typed structurally, over every scalar field. A root read keeps the
per-entity `Filters`/`Order` types, so an unknown field there stays a compile
error; only a branch is checked at run time.

What counts as a scalar is decided by `Scalar`, which reads the `Brand`
that `@primitive` already puts on every primitive:

```ts
type Scalar<T> =
    NonNullable<T> extends string | number | boolean | bigint | Date | Brand<any> ? true : false;
```

The brand is load-bearing. A structural `extends object` test cannot tell a
scalar from an entity: `Money`, `WidgetId`, and `Version` are branded
primitives and all satisfy `extends object`, exactly like `WidgetRow`. Without
the brand, `select: { subtotal: { … } }` would type-check and recurse into a
decimal.

## A read, generated

A read takes **two arguments**: the executor `db` first, then an options object
that carries the filter and the selection. The generator keeps them as separate
keys — `{ filter, select }` — so a data field can never collide with `select`,
and passes the filter object straight to the resolver, whose signature already
keeps filters and projection explicit.

A call site narrows exactly:

```ts
const [widget] = await queryWidget(db, {
    filters: [{ tenantId__eq: tenantId }, { id__eq: id }],
    select: {
        id: true,
        code: true,
        subtotal: true,
        rows: { select: { description: true, subtotal: true } },
    },
});
// widget.rows![0].subtotal      ✓
// widget.rows![0].surcharge      ✗  not selected
// widget.notes                ✗  not selected
```

The compiler rejects a field that is not on the entity, a nested selection on a
scalar, a field not named in the selection, a branch without a `select`, a
non-array filter, and a root read that leaves out a required filter.

The generator emits the **contract and the wiring**, never SQL. It reads only
annotations: `@queryFilter` for the filters, the existing table model for the
columns, and the branch tags below. No domain name is hardcoded, so a new
`@queryFilter` field and a new entity flow through with no generator edit.

## What is generated, and what is not

| Artifact | Source |
| --- | --- |
| `query<Entity>` functions | generated |
| `queryModel` (tables, fields, branches) | generated |
| `resolveMany` | hand-written, once |

`queryModel` is the physical model: for each table its key, its scalar fields,
and a descriptor per branch (see below). The resolver reads that data plus the
runtime `args`/`select`, and builds the SQL. The generated functions carry no
logic — they name a table as a string and delegate.

## Selecting a branch means three different things

`Widget.rows`, `Widget.account`, and `WidgetSnapshot.account` nest identically
at the call site and are not the same storage:

| Nested field | Annotation | Storage | SQL |
| --- | --- | --- | --- |
| `Widget.rows` | `@children` | child table, FK `widgetId` | join |
| `Widget.account` | `@relation` | FK `accountId` | join to the **live** row |
| `WidgetSnapshot.account` | `@inlined` | flattened `accountName`, `accountEmail`, … | no join, prefix columns |

An inlined branch is not a join: its columns are already on the row. The
freshness differs too — a relation reads the current record, while an inlined
value is copied from a snapshot entity into the row at write time — and the call
site cannot tell them apart. An inlined branch still takes a `select`, but its
other read arguments have nowhere to apply: there is no separate statement to
filter, order, or page, so they are ignored for an inlined branch.

The generator records the distinction in `queryModel`, from the branch tags the
table model already carries (`scripts/infra/postgres-model.ts`):

```ts
account:  { kind: "relation", table: "account", column: "accountId" }
rows:     { kind: "children", table: "widget_row", column: "widgetId" }
snapshot: { kind: "inlined", columns: { name: "accountName" } }
```

The resolver types these as a union on `kind`, so a branch to another table always
carries its `table` and `column`, and an inlined one only its `columns`.

Relation and inlined columns are not the same here. Inlined columns are excluded
from the table's selectable `fields`, because they are not fields of the
interface. A relation's foreign key *is* excluded from nothing: it is the
`@foreignKey` field the interface declares, so `Widget.accountId` is a selectable
field and — with `@queryFilter` — an ordinary filter. `WidgetRow.widgetId` is the
same thing.

## The resolver reads one query per branch

`resolveMany` (`packages/infra/src/db/resolvers.ts`) builds the SQL at run
time from `queryModel` plus the selection:

- **Scalars** — projected into the root query.
- **`@inlined`** — projected from the same row, aliased per target field. No query.
- **`@relation`** — one batched `where target.key = any($n)` for every distinct
  foreign key, attached back by key.
- **`@children`** — one batched `where child.fk = any($n)`, grouped in memory.

A branch's own arguments join that statement: its `filters` add comparisons on
the target's columns, and its `order` sorts the batch before it is grouped. Its `limit`/`offset` bound each
parent, which needs a ranked subquery rather than a plain `limit` — the resolver
ranks the branch rows over the parent key and keeps each parent's window. A
branch with no `select` throws, as does a filter, order, or comparison the target
does not allow.

A projected scalar is copied only when its column is not `null`, and a to-one
branch whose foreign key is `null` is left out. A `null` column and an absent
key therefore look the same on the wire: a value is present exactly when the
column has one, so an optional field may be absent and a required one never is.
See `docs/optionality.md`.

There is **no JSON aggregation**. A branch costs one extra query, batched over
all parents at that level; nested branches recurse the same way, one query per
branch per level. The resolver holds no domain knowledge — it only reads
`queryModel`, so a new entity and `@queryFilter` field need no resolver change.

Arguments filter by set membership: the generated filters are the scalar fields
that carry `@queryFilter` (plus every `@primaryKey` field), and a relation's
`@foreignKey` field is one of them. An unknown filter field throws rather than
silently dropping a clause, as does a non-array value, and a filter marked
`required` throws when the read does not name it. `String`, `Date`, and
`decimal` values pass through unchanged.

A filter value is always an array, so a lookup names a one-element set and an
empty set matches nothing. Each set is bound as a single array parameter, so a
batched branch over any number of parents still carries one parameter for its keys,
and Postgres infers the array's element type from the column. Multiple filters are ANDed. The linter rejects
`@queryFilter` on a branch field, so a filter always names a scalar column.

Ordering, paging, and comparisons are applied the same way: the resolver reads
the table's `order`/`defaultOrder` and operator whitelists from `queryModel`,
refuses a column or operator that is not on them, and validates the direction and
operator before they reach the SQL, where every value is parameterized. A root
read's `limit` defaults to 1000 and its `offset` to 0, and a value that is not a
positive integer or a non-negative integer is rejected; a branch's `limit`/`offset`
are optional and bound each parent. A branch keeps the batch's grouping stable,
because the ranked window orders each parent's rows before they are grouped.

## Gotchas

- **A selected key keeps the optionality it has on the entity.** A required
  scalar selected with `true` stays required; an optional field stays optional
  because its column is nullable and the resolver omits a `null` rather than
  carrying it. Selection never adds `?` to a required field. See
  `docs/queries.md`.
- **`true` on a branch selects scalars only.** It does not recurse into nested
  branches, so a default selection cannot fan out into unbounded joins.
- **A branch is `true` or a read's arguments.** An object branch requires a
  `select`, so `rows: {}` is invalid; `rows: { select: { description: true } }` is
  how a branch is narrowed. On an **inlined** branch only `select` applies — there
  is no separate statement to filter, order, or page.
- **A branch's facets are typed structurally, checked at run time.** The branch's
  `filters` and `order` accept every scalar field of the target, because a
  hand-written `Selection` cannot read the target's annotations. The resolver (and
  the generated args schema, over HTTP) still refuses a field or operator the
  target does not allow, so an unannotated one is a throw or a 400 rather than a
  compile error. A root read keeps the exact `Filters`/`Order` types.
- **A filter reads columns the selection may omit.** `select` governs the
  projection; the `filters` still read whatever they name, and so does `order`.
- **One entry is one comparison.** `filters` is a list of entries combined with
  `and`, each naming one column with one operator, so a column is filtered twice
  by two entries. An entry naming two columns throws.
- **A group is an entry, and its branches are entries.** `{ kind: "or", filters:
  […] }` matches when any branch does, `{ kind: "and", filters: […] }` when all do,
  and a branch may itself be a group, so the tree nests to any depth.
- **A range is two entries, or one `between`.** `ops=` decides which is offered:
  `{ gte }` and `{ lte }` need no `between`, and `between` needs no second entry.
  Both render the same rows.
- **Filters come only from `@queryFilter`, plus the primary key.** A field
  without the tag is not filterable, each `@primaryKey` field is a filter without
  it, and a branch field may not carry it. A parent is not filtered by a related
  record's columns, such as `account.name`; a branch's own `filters` narrow the
  branch, and the foreign key is a scalar field, so `accountId` is filterable.
- **A default fills a hole, it does not narrow by itself.** `@queryFilter
  default=<value>` applies when a read omits the filter; a read that names it
  overrides the declared value. This is how a nullable `deletedAt` reads live rows
  by default (`docs/soft-delete.md`).
- **A composite key has no single row key for branch attachment.** The resolver
  identifies a row by one column, so an entity whose `@primaryKey` is several
  fields cannot be the target of a `@foreignKey` — and so cannot be reached by a
  `@relation` or own a `@children` collection, both of which need one column to
  point at it. The generator reports that rather than emitting a partial join.
  Such an entity is therefore root-only, and `queryModel.key` names its first key
  column purely as the alias a row is projected under; no join reads it, because
  nothing can name the table as a branch target. Grouping children by a full
  composite key is not implemented.
- **The brand is an internal.** `Scalar` reads the `Brand` alias, whose
  only dependency is a type-only Zod import. The fallback, if that ever moves, is
  separate `select` (scalars) and `with` (branches) keys, which needs no brand
  but reads worse.
- **A branch `true` on an entity with no relations is safe; a cycle is not.**
  `WidgetRow.widgetId` is a scalar alias, not a `@relation`, so the
  recursion terminates. A `@relation` back to the parent would need a depth cap
  in both `Selection` and the resolver.
- **Ordering comes only from `@queryOrderBy`.** A field without the tag cannot
  order, and a direction is `asc` or `desc`. The tag is scalar-only, so a branch
  field may not carry it; a branch orders by its own entity's fields, through the
  same whitelist.
- **A root read is capped by default, a branch is not.** `limit` defaults to 1000
  on a root read, so a filter that matches more rows returns the first 1000 unless
  the caller asks for more. There is no `limit: null` for "all rows"; a root page
  is always bounded, while a branch returns every matching row unless it names its
  own `limit`.
- **`offset` is not a cursor.** A concurrent insert or delete can shift a row
  across pages between calls. Stable paging over a mutable table needs a keyset
  cursor, which is not implemented.
- **The read types are shared, the reader is not.** `Selection`, `Selected`,
  `QueryArgs`, `Filters`, `Order`, and `Where` are pure types with no query in
  them, so they live in `packages/sdk/src/selection.ts` and both infra and the
  generated REST client import them from there. The SQL, the resolver, and the
  metadata stay in infra.

## Why the resolver is hand-written

The SELECT depends on the runtime `select` object, and the WHERE on the runtime
`args`; neither is known at build time, so a generator cannot emit the SQL the
way `generate-repositories.ts` emits an INSERT. What the generator *can* emit is
mechanical: the per-entity functions and the physical model. The interpreter
is written once, holds no domain names, and reads generated data — which is what
`AGENTS.md` asks for. Generating a resolver per entity would copy identical
logic N times.

Writing the WHERE per query is the alternative — the args shape is known at
build time, so a generator could emit `[["accountId", "=", args.accountId]]`
and leave only projection and shaping to the resolver. That is a reasonable
middle ground if the args logic grows past equality.

## Deliberately not implemented

- **Comparing or filtering a parent by a branch's columns.** `@queryFilter` is
  scalar-only, so `queryWidget` cannot compare or filter on a related record's
  columns, such as `account.name`. A branch's own `filters` narrow the branch, not
  the parent; the foreign key is a scalar field, so `accountId` filters the parent.
- **Comparisons across fields.** A filter value is a literal, not a column
  reference, so `startsAt < endsAt` is not expressible.
- **A row at most, not exactly one.** There is no `limit 1` shorthand; a caller
  takes the first result, which fetches a whole page.
- **Cursor pagination and a total count.** `limit`/`offset` page a read, but
  there is no keyset cursor and no way to ask how many rows matched.
- **`@projection` generation.** If static read SQL is ever wanted, the hook is a
  `@projection` tag on a `Pick<Entity, "…">` alias, emitting a view or a column
  list. It is not built.
- **Returning a whole entity in one call.** There is no `select: true` shorthand
  for the entire interface; a caller names the fields.
- **A transaction around a multi-query read as a property of a read.** A batched
  branch and its root are separate statements, so a concurrent write can be
  observed between them; a caller that needs the two to agree wraps them in a
  `transaction` group, which a read does not do on its own
  (`docs/transactions.md`).
