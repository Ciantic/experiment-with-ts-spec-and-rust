# Optionality

A field's `?` mirrors its column's nullability, and nothing else:

- **`not null` column → required field.** No `?`.
- **Nullable column → optional field.** `?`.

The spec is the shape a caller reads, and reading is what most of the
application does. A `not null` column that appears optional in the spec is a lie
the type system repeats at every call site: the caller has to prove a value is
there that was never absent.

## The database filling a column does not make the field optional

Several tags own a column's value, so the column is `not null` whatever the
field says:

- `@primaryKey` — the key names the row, so it is never absent.
- `@createdAt` and `@updatedAt` — `default now()`, and a generated write falls
  back to it. See `docs/timestamps.md`.
- `@version` — `default 0` supplies the first revision. See `docs/versioning.md`.
- `@pgAutoIncrement` — the database assigns the key. See `docs/auto-increment.md`.

The column is `not null` because a value is guaranteed, so the field is
required. The field says what the row has; the tags say what a write does when
the field is absent.

`@pgDefault` is not in that list. It fills a column a statement omits, but it
leaves an explicit `null` alone, so it changes what a write does rather than
whether the column can be null:

- On a **required** field the column is `not null`, and the expression is the
  value an omitted field gets. A `UUIDv7` column is the common case, since the
  type supplies `uuidv7()` unless the field overrides or clears it. See
  `docs/primitives.md`.
- On an **optional** field the column stays nullable, and the expression is only
  the fallback for an omission. The three states are distinct: omit takes the
  default, `null` writes a null, and a value writes the value.

## The create is a different shape

Required in the spec does not mean required on the wire. A create relaxes each
field whose column has a fallback:

- A clock column and a defaulted `@version` stay in `<Entity>Insert`, but
  optional: the schema marks them `.exactOptional()` and the statement writes the supplied
  value, or the column default when the row omits it.
- A `@pgDefault` field is relaxed the same way.

So a defaulted `status` column is required in its entity and optional in
`MessageInsert`, without one contradicting the other. The same applies to `createdAt`, `updatedAt`, and a
defaulted `version`, which a seed or a backfill may supply. See
`docs/validation.md` and `docs/repositories.md`.

A nullable field that also has a default is the one place a create carries three
states rather than two: omitting it takes the default, `null` writes a null, and
a value writes the value. The generated write type says so — `T | null` rather
than `T` — and the statement tests the field's presence before choosing between
the value and the default.

## What stays optional

A field whose column is nullable, which is how `@computed` values start life:

- `Widget.subtotal`, `Widget.surcharge`, `Widget.total` — computed from
  the rows, so absent until there are rows. A trigger propagates `null` rather
  than inventing a zero.
- `WidgetRow.subtotal` and friends, the same way.
- Plain optional data: `Account.externalId`, `Widget.notes`. Nullable, because
  an account may have no external identifier.

## Writing one back

The read type is `T | undefined`, and a write takes the same two states meaning
different things. A create writes a nullable column whether the caller omits the
field or sends it, so `undefined` and `null` are one request there — unless the
column also has a default, where omitting it takes the default and `null` writes
a null. A patch always tells them apart: omitting the field leaves the stored
value alone, and `null` clears the column, which is the only way to write a null
(`docs/repositories.md`).

An upsert takes both like a create and stores them like one — it writes the row
whole, so omitting the field and sending `null` are again one request unless the
column has a default. It accepts `null` anyway, because a caller holding a row
from a form or another JSON surface carries `null` where a spec may carry an
absent field.

## Generation

`scripts/infra/postgres-model.ts` makes a column `not null` from the field's `?`
and from the tags that own the value, never from `@pgDefault`, and a generator
reads the tag, never the field name — so adding an entity never edits a
generator. A field is therefore optional exactly when its column is nullable.

`lint:spec` checks the direction a tag can force: a tag that owns its column —
`@primaryKey`, `@createdAt`, `@updatedAt`, `@version`, `@pgAutoIncrement` — may
not sit on an optional field. The other direction is not reported, since no lint
rule reads the DDL and the interface together: a required field on a nullable
column passes.

## Gotchas

- **A selected key keeps the optionality it has on the entity.** A required
  scalar selected with `true` is required in the result; an optional field stays
  optional because its column is nullable and the resolver omits a `null`. See
  `docs/queries.md`.
- **A `null` column and an absent key look the same on the wire.** The resolver
  drops a `null` scalar, so an optional field's value is missing rather than
  `null`; a required field's never is.
- **Mock data declares the required fields anyway.** A row is typed as the
  entity, so it carries `createdAt`, `updatedAt`, and `version`, and the
  generated insert stores the declared values. See `docs/mockdata.md`.
