# Rust models

`scripts/models-rs/generate-models.ts` writes the entities and primitives into
`crates/models/src/domain/`. The output mirrors the spec's directory structure:
an entity at `spec/src/widget-service/Widget.ts` becomes
`domain/widget-service/widget.rs`, and a primitive beside it stays beside it.

A directory whose name is not a Rust identifier keeps its spec name on disk and
is declared through `#[path]`, so `widget-service/` is the `widget_service`
module.

## The six shapes of an entity

One entity file holds six structs, each derived from the same annotations the
TypeScript write types read (`docs/validation.md`):

| Struct | Purpose | Field set |
| --- | --- | --- |
| `Widget` | the row as read | every field |
| `WidgetSelect` | the row as selected | every field |
| `WidgetInsert` | what a create writes | the create's omit list removed |
| `WidgetUpsert` | what an upsert writes | the create's, plus the key it claims |
| `WidgetPatch` | what an update writes | the patch's omit list removed |
| `WidgetPrimaryKey` | what a delete addresses | the key columns |

`WidgetPrimaryKey` is written only when the entity declares a `@primaryKey`.

The field sets come from `scripts/spec/spec-model.ts`, so a Rust struct cannot
drift from the schema the same entity generates in `packages/sdk`:
`omittedFromInsert`, `defaultedInsertProperties`, `inlinedFromInsert`,
`omittedFromPatch`, `nullablePatchProperties`, and `primaryKeyProperties`.

### The row

A struct named after the interface, with snake-cased fields and
`#[serde(rename_all = "camelCase")]`, so the wire shape matches the spec. An
optional field becomes `Option<T>` with
`#[serde(default, skip_serializing_if = "Option::is_none")]`.

### The selected row

`WidgetSelect` makes every field presence-tracked, so a read says which fields
it selected and the decoder says which arrived:

```rust
pub struct WidgetSelect {
    #[serde(default, skip_serializing_if = "Partial::is_unset")]
    pub id: Partial<WidgetId>,
    #[serde(default, skip_serializing_if = "Partial::is_unset")]
    pub notes: Partial<Option<String>>,
    #[serde(default, skip_serializing_if = "Partial::is_unset")]
    pub rows: Partial<Option<Vec<WidgetRowSelect>>>,
}
```

An unselected field is `Unset`; a selected one is `Set(value)`. A nullable field
nests `Option`, so `Set(None)` distinguishes a selected null from an omitted
field; the resolver drops a `null` scalar today (`docs/optionality.md`), so that
state is accepted rather than emitted. A branch carries its target's own
selection, so a nested read is typed too.

### The write shapes

`WidgetInsert`, `WidgetUpsert`, and `WidgetPatch` mirror the create, upsert, and
patch schemas, including their `.strict()` — each carries
`#[serde(deny_unknown_fields)]`, so a field the write would ignore is a decode
error rather than a silently dropped key.

- **Create.** A branch (`@relation`, `@children`) and a column the database owns
  (an identity key, a virtual generated column, a nullable computation) are
  absent. A field whose column has a fallback — `@pgDefault`, `@createdAt`,
  `@updatedAt`, or a defaulted `@version` — is `Option<T>`, because omitting it
  takes the column default; a nullable field that also has a fallback is
  `Partial<Option<T>>` instead, so a null stays expressible. See "Null and
  absence in a create".
- **Upsert.** The create's set, with the key and the `@version` required: an
  upsert claims the row it replaces. An identity key is named here even though a
  create leaves it out, so the conflict has a target.
- **Patch.** Every written field is `Partial<T>`, so an absent field leaves the
  stored value alone, and a nullable one is `Partial<Option<T>>`, so `Set(None)`
  clears the column. The key and the `@version` stay plain and required; the
  creation clock and every branch are absent.

An `@inlined` field nests its target's write shape — `Option<AccountInsert>` —
so the snapshot columns can only be given values a write would store.

### Null and absence in a create

A create writes a nullable column whether the caller omits the field or sends a
`null`, so the two are one request (`docs/optionality.md`). The mapping follows
that: a field that can be omitted and either be null or take a column default has
three states, and a field without both of those has two.

| Field | Create type | `Unset` / `None` | `Set(None)` | Value |
| --- | --- | --- | --- | --- |
| required, no default | `T` | — | — | the value |
| required, defaulted | `Option<T>` | omit, take the default | — | the value |
| nullable, no default | `Option<T>` | omit, which writes `NULL` | — | the value |
| nullable, defaulted | `Partial<Option<T>>` | omit, take the default | write `NULL` | the value |

Only the last row distinguishes omission from a null, so only it uses `Partial`:
the two requests write different rows, and collapsing them would make the column's
`NULL` unreachable from a create.

The executor reads the column, not the Rust value, when it decides what an absent
field means: a defaulted column binds a per-row presence flag so an omitted one
takes its default, and a nullable column without a default binds `NULL` directly.

### The key

`WidgetPrimaryKey` holds the `@primaryKey` fields and nothing else, so a
composite key names every column and a caller cannot address a row with a
partial key.

## Scalar mapping

The mapping follows the type's storage type, so a named alias and a bare keyword
agree:

| Spec | Rust |
| --- | --- |
| `string` | `String` |
| `number` | `f64` |
| `bigint` | `i64` |
| `boolean` | `bool` |
| `Date` | `chrono::DateTime<chrono::Utc>` |
| `Uint8Array` | `Vec<u8>` |
| `Record` | `serde_json::Value` |
| `T[]` | `Vec<T>` |

## Primitives and aliases

A primitive carrying `@primitive`, and every alias declared in an entity file,
becomes one of three items:

- A union of string literals becomes an enum with `#[serde(rename = "...")]` on
  each variant. When the union also admits any other string, the enum carries an
  `Other(String)` variant and a hand-written serde implementation that preserves
  the unknown value.
- An object literal becomes a struct, with its members rendered as fields.
- Any other type becomes a newtype over its base: `WidgetId` becomes
  `pub struct WidgetId(pub String);` and `Money` over `Decimal` becomes
  `pub struct Money(pub Decimal);`. A brand is a nominal distinction in
  TypeScript, and a newtype keeps that distinction in Rust.

A generic alias such as `BrandedId<Name>` has no Rust spelling without a marker
type, so it is not written. The aliases that name it are written as their own
newtypes, which is the shape the entities use.

## Not generated yet

A selection *request* — what a caller sends to say which fields it wants — is a
different shape from `WidgetSelect`: it carries `true` per scalar and nested
arguments per branch, not a value. It belongs with the query arguments
(`filter`, `order`, `where`, `limit`, `offset`), which are not generated.
