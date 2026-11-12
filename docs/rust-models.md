# Rust models

`scripts/models-rs/generate-models.ts` writes the entities and primitives into
`crates/models/src/domain/`. The output mirrors the spec's directory structure:
an entity at `spec/src/widget-service/Widget.ts` becomes
`domain/widget-service/widget.rs`, and a primitive beside it stays beside it.

A directory whose name is not a Rust identifier keeps its spec name on disk and
is declared through `#[path]`, so `widget-service/` is the `widget_service`
module.

## Entities

An entity becomes a struct named after the interface, with snake-cased fields
and `#[serde(rename_all = "camelCase")]`, so the wire shape matches the spec.
An optional field becomes `Option<T>` with
`#[serde(default, skip_serializing_if = "Option::is_none")]`. A field whose type
is declared in another spec file is imported from that file's module.

The mapping of a scalar follows the type's storage type, so a named alias and a
bare keyword agree:

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
