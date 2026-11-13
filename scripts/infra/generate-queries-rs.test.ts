/** Unit tests for the Rust query generator, driven by self-contained fixtures. See docs/testing.md. */
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SPEC_SRC_ROOT, parseInMemorySpec } from "../spec/spec-model.ts";
import type { Column, Table } from "./postgres-model.ts";
import {
    fieldEnumName,
    generateQueryModules,
    queryFileName,
    queryFunctionName,
    renderQueryModule,
    selectTypePath,
    selectableFields,
    sqlTypeVariant,
} from "./generate-queries-rs.ts";

/** The glob that matches an in-memory fixture, placed so its import specifier looks like a real one. */
const SPEC_GLOB = join(SPEC_SRC_ROOT, "**/*.ts");

/** The types the fixture's columns carry, so a filter can be checked against the model's own type. */
const spec = parseInMemorySpec(
    [
        {
            filePath: join(SPEC_SRC_ROOT, "fixtures.ts"),
            sourceFileText: `
/**
 * @primitive
 * @pgType uuid
 */
export type InvoiceId = string;

/**
 * @primitive
 * @pgType uuid
 */
export type TenantId = string;

/**
 * @entity
 * @pgTable invoice
 * @queries query
 */
export interface Invoice {
    /** @primaryKey */
    id: InvoiceId;
    /** @queryFilter */
    tenantId: TenantId;
    number?: string;
    updatedAt: Date;
    /** @queryFilter */
    deletedAt?: Date;
    /** @queryFilter ops=gte,lte */
    issueDate?: Date;
    customerName: string;
}

/**
 * @entity
 * @pgTable audit_log
 * @queries query
 */
export interface AuditLog {
    /** @primaryKey */
    id: string;
}
`,
        },
    ],
    { sourceGlob: SPEC_GLOB },
);

function column(name: string, extras: Partial<Column> = {}): Column {
    return { name, sqlType: "text", notNull: true, primaryKey: false, unique: false, insertable: true, updatable: true, ...extras };
}

function table(name: string, interfaceName: string, columns: Column[], relations: Table["relations"] = new Map()): Table {
    return {
        name,
        interfaceName,
        importSpecifier: `spec/fixtures/${interfaceName}.ts`,
        columns,
        relations,
        triggers: [],
        repositoryOperations: [],
        queries: ["query"],
    };
}

const invoice = table(
    "invoice",
    "Invoice",
    [
        column("id", { sqlType: "uuid", primaryKey: true, queryFilter: { ops: ["eq"] } }),
        column("tenantId", { sqlType: "uuid", queryFilter: { ops: ["eq"] } }),
        column("number", { queryOrder: { default: "asc" } }),
        column("updatedAt", { sqlType: "timestamptz", queryOrder: {} }),
        column("deletedAt", { sqlType: "timestamptz", notNull: false, queryFilter: { ops: ["eq"] } }),
        column("issueDate", {
            sqlType: "timestamptz",
            notNull: false,
            queryFilter: { ops: ["gte", "lte", "between"] },
        }),
        column("customerName"),
    ],
    new Map([["snapshot", { kind: "inlined", table: "customer", columns: { name: "customerName" } }]]),
);

const bare = table("audit_log", "AuditLog", [column("id", { primaryKey: true })]);

const nullable = table("login_link", "LoginLink", [
    column("id", { queryFilter: { ops: ["eq"] } }),
    column("consumedAt", { sqlType: "timestamptz", notNull: false, queryFilter: { ops: ["isNull", "isNotNull"] } }),
]);

describe("queryFunctionName", () => {
    it("snake-cases the entity", () => {
        expect(queryFunctionName("InvoiceSentRow")).toBe("query_invoice_sent_row");
        expect(queryFileName("InvoiceSentRow")).toBe("query_invoice_sent_row.rs");
    });
});

describe("selectTypePath", () => {
    it("points at the model's selected shape", () => {
        expect(selectTypePath(invoice)).toBe("models::domain::fixtures::invoice::InvoiceSelect");
    });
});

describe("sqlTypeVariant", () => {
    it("maps a spacing of the same Postgres type to one variant", () => {
        expect(sqlTypeVariant("uuid")).toBe("Uuid");
        expect(sqlTypeVariant("integer")).toBe("I32");
        expect(sqlTypeVariant("timestamp with time zone")).toBe("Timestamptz");
    });

    it("falls back to text for an unlisted type", () => {
        expect(sqlTypeVariant("tsvector")).toBe("Text");
    });
});

describe("selectableFields", () => {
    it("keeps every column a branch does not own", () => {
        expect(selectableFields(invoice)).toEqual(["id", "tenantId", "number", "updatedAt", "deletedAt", "issueDate"]);
    });
});

describe("fieldEnumName", () => {
    it("names one enum per capability", () => {
        expect(fieldEnumName("Invoice", "Select")).toBe("InvoiceSelectField");
        expect(fieldEnumName("Invoice", "Order")).toBe("InvoiceOrderField");
    });
});

describe("renderQueryModule", () => {
    it("names the entity's own query type, not the generic read options", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub struct InvoiceQuery {");
        expect(code).toContain("pub select: InvoiceSelection,");
        expect(code).toContain("pub filters: Vec<InvoiceFilter>,");
        expect(code).toContain("pub order: Vec<InvoiceOrder>,");
        expect(code).toContain("pub fn as_sql(&self) -> Result<PlannedQuery, ReadError> {");
        expect(code).toContain(
            "    pub async fn query(self, db: &impl SqlExecutor) -> Result<Vec<InvoiceSelect>, ReadError> {",
        );
    });

    it("converts the typed selection and ordering to the builder's own types", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub(crate) fn into_read_options(self) -> Result<ReadOptions, ReadError> {");
        expect(code).toContain(".map(|field| field.select())");
        expect(code).toContain("filters.extend(filter.into_typed_filters()?);");
        expect(code).toContain(".map(|order| order.into_sort())");
        expect(code).toContain('read.plan("invoice", None)');
    });

    it("builds the selection, filters, and ordering with a small fluent API", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub fn with_select(fields: impl IntoIterator<Item = InvoiceSelectField>) -> Self");
        expect(code).toContain("pub fn filter(mut self, filter: impl Into<InvoiceFilter>) -> Self");
        expect(code).toContain("pub fn filter_or(");
        expect(code).toContain("pub fn filter_and(");
        expect(code).toContain("pub fn order(mut self, order: InvoiceOrder) -> Self");
    });

    it("plans the read from the builder, so a caller can log the plan before running it", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("        let read = self.clone().into_read_options()?;");
        expect(code).toContain('        read.plan("invoice", None)');
    });

    it("runs the read from the builder, binding its own parameters", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("        let read = self.into_read_options()?;");
        expect(code).toContain('        let query = read.plan("invoice", None)?;');
        expect(code).toContain("        let rows = db.query(&query.sql, &query.params).await?;");
        expect(code).toContain("        decode_rows(rows)");
    });

    it("wraps a query signature the entity's selected type makes too long", () => {
        const long = table("tenant_shared_permission", "TenantSharedPermission", [column("id", { primaryKey: true })]);
        const code = renderQueryModule(long, spec);

        expect(code).toContain(
            "    pub async fn query(\n        self,\n        db: &impl SqlExecutor,\n    ) -> Result<Vec<TenantSharedPermissionSelect>, ReadError> {",
        );
        expect(code).toContain('        let query = read.plan("tenant_shared_permission", None)?;');
    });

    it("accepts the same column twice, since the list is combined with and", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("self.filters.push(filter.into());");
        expect(code).toContain("The same column may be filtered twice.");
        expect(code).toContain("pub filters: Vec<InvoiceFilter>,");
    });

    it("writes each comparison as a flat `column__operator` key, as the TypeScript read does", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain('flat_filter::serialize("tenantId__eq", value, serializer)');
        expect(code).toContain('"tenantId__eq" => Ok(Self::Eq(flat_filter::payload(key, value)?))');
        // The umbrella enum is untagged, so a column variant is the entry itself.
        expect(code).toContain("#[serde(untagged)]");
        expect(code).not.toContain('#[serde(rename_all = "camelCase")]');
    });

    it("writes a range as the two-element array its operator takes", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain('flat_filter::serialize("issueDate__between", &[lower, upper], serializer)');
        expect(code).toContain("let (lower, upper) = flat_filter::payload(key, value)?;");
    });

    it("writes a group with the kind discriminator the TypeScript read sends", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("    Group {");
        expect(code).toContain("kind: FilterKind,");
        expect(code).toContain("kind: FilterKind::Or,");
        expect(code).toContain("kind: FilterKind::And,");
    });

    it("writes an ordering clause as a `[column, direction]` tuple", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("let mut tuple = serializer.serialize_tuple(2)?;");
        expect(code).toContain("tuple.serialize_element(self.field.as_str())?;");
        expect(code).toContain("tuple.serialize_element(&self.direction)?;");
        // Nulls ordering has no wire spelling, so the clause is a plain pair.
        expect(code).not.toContain("pub fn nulls_first");
        expect(code).not.toContain("nulls: Option<NullsOrder>,");
    });

    it("writes the selection as an object naming each column", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub struct InvoiceSelection(pub Vec<InvoiceSelectField>);");
        expect(code).toContain("map.serialize_entry(field.as_str(), &true)?;");
        expect(code).toContain("flat_filter::selection(deserializer, InvoiceSelectField::from_column).map(Self)");
    });

    it("gives each filterable column an enum of exactly its ops", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub enum InvoiceTenantIdFilter {");
        expect(code).toContain("    Eq(TenantId),");
        expect(code).not.toContain("pub tenant_id: Option<");
    });

    it("names each column's type when it lowers to the planner", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain(
            'TypedFilter::one("tenantId", SqlType::Uuid, FilterOperator::Eq, value)?',
        );
        expect(code).toContain(
            'TypedFilter::one("deletedAt", SqlType::Timestamptz, FilterOperator::Eq, value)?',
        );
    });

    it("lets a where column carry exactly its whitelisted operator", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub enum InvoiceIssueDateFilter {");
        expect(code).toContain("Gte(chrono::DateTime<chrono::Utc>),");
        expect(code).toContain("Lte(chrono::DateTime<chrono::Utc>),");
        expect(code).not.toContain("Gt(chrono::DateTime<chrono::Utc>),");
        expect(code).toContain("FilterOperator::Gte,");
    });

    it("converts every column type into the umbrella filter, so filter() takes it directly", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("impl From<InvoiceTenantIdFilter> for InvoiceFilter {");
        expect(code).toContain("Self::TenantId(filter)");
        expect(code).toContain("impl From<InvoiceIssueDateFilter> for InvoiceFilter {");
    });

    it("groups a branch's filters by its own kind, and leaves one branch unparenthesised", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("Self::Group { kind, filters } => {");
        expect(code).toContain("_ => vec![TypedFilter::group(kind.combinator(), typed)],");
        expect(code).toContain("1 => typed,");
        expect(code).toContain("0 => Vec::new(),");
    });

    it("serializes each field enum as its column name", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("#[derive(");
        expect(code).toContain("Clone, Copy, Debug, serde::Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize,");
        expect(code).toContain('    #[serde(rename = "tenantId")]\n    TenantId,');
    });

    it("serializes the query as camelCase JSON, rejecting an unknown key", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain(
            "#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq, serde::Serialize)]",
        );
        expect(code).toContain('#[serde(rename_all = "camelCase", deny_unknown_fields)]\npub struct InvoiceQuery {');
        // The TypeScript read always sends a selection, so a read always writes one, even empty.
        expect(code).toContain("#[serde(default)]\n    pub select: InvoiceSelection,");
        expect(code).toContain('#[serde(default, skip_serializing_if = "Option::is_none")]\n    pub limit: Option<u64>,');
    });

    it("names an ordering clause's column through its field set, and its direction through the planner's own", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("impl<'de> serde::Deserialize<'de> for InvoiceOrder {");
        expect(code).toContain("let (field, direction) = flat_filter::sort(");
        expect(code).toContain("Ok(Self { field, direction })");
    });

    it("keeps the read on the builder, with no free function beside it", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).not.toContain("pub async fn query_invoice(");
        expect(code).not.toContain("pub fn build_query_invoice(");
    });

    it("emits an enum per field set with the columns it names", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub enum InvoiceSelectField {");
        expect(code).toContain("pub enum InvoiceOrderField {");
        expect(code).toContain("Self::TenantId => \"tenantId\",");
    });

    it("builds filters, selects, and sorts from the typed fields", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub fn select(self) -> SelectColumn");
        expect(code).toContain("pub fn asc(self) -> InvoiceOrder {");
        expect(code).toContain("pub struct InvoiceOrder {");
    });

    it("writes an empty field set when the entity declares none", () => {
        const code = renderQueryModule(bare, spec);

        expect(code).toContain("pub enum AuditLogOrderField {}");
        expect(code).toContain("pub const ALL: &'static [Self] = &[];");
    });

    it("writes a null test as a flat entry holding `true`, and rejects any other payload", () => {
        const code = renderQueryModule(nullable, spec);

        expect(code).toContain('Self::IsNull => flat_filter::serialize("consumedAt__isNull", &true, serializer),');
        expect(code).toContain('"consumedAt__isNotNull" => {');
        expect(code).toContain("flat_filter::null_test(key, value)?;");
    });

    it("wraps a long with_select signature instead of overflowing the line", () => {
        const long = table("tenant_shared_permission", "TenantSharedPermission", [column("id", { primaryKey: true })]);
        const code = renderQueryModule(long, spec);

        expect(code).toContain("pub fn with_select(\n        fields: impl IntoIterator<Item = TenantSharedPermissionSelectField>,\n    ) -> Self {");
    });
});

describe("generateQueryModules", () => {
    it("emits one module per reading entity and a barrel", () => {
        const seller = table("seller", "Seller", [column("id", { primaryKey: true })]);
        const files = generateQueryModules(new Map([
            ["Invoice", invoice],
            ["Seller", seller],
        ]), spec);

        expect([...files.keys()].sort()).toEqual(["mod.rs", "query_invoice.rs", "query_seller.rs"]);
    });

    it("omits an entity that declares no @queries", () => {
        const seller: Table = { ...table("seller", "Seller", [column("id", { primaryKey: true })]), queries: [] };
        const files = generateQueryModules(new Map([
            ["Invoice", invoice],
            ["Seller", seller],
        ]), spec);

        expect([...files.keys()].sort()).toEqual(["mod.rs", "query_invoice.rs"]);
        expect(files.get("mod.rs")).not.toContain("query_seller");
    });

    it("re-exports every module from the barrel", () => {
        const code = generateQueryModules(new Map([["Invoice", invoice]]), spec).get("mod.rs") ?? "";

        expect(code).toContain("mod query_invoice;");
        expect(code).toContain("pub use query_invoice::*;");
    });
});
