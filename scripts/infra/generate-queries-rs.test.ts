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
    /** @queryWhere gte lte */
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
        column("id", { sqlType: "uuid", primaryKey: true, queryFilter: { required: false } }),
        column("tenantId", { sqlType: "uuid", queryFilter: { required: false } }),
        column("number", { queryOrder: { default: "asc" } }),
        column("updatedAt", { sqlType: "timestamptz", queryOrder: {} }),
        column("deletedAt", { sqlType: "timestamptz", notNull: false, queryFilter: { required: false } }),
        column("issueDate", { sqlType: "timestamptz", notNull: false, where: ["gte", "lte"] }),
        column("customerName"),
    ],
    new Map([["snapshot", { kind: "inlined", table: "customer", columns: { name: "customerName" } }]]),
);

const bare = table("audit_log", "AuditLog", [column("id", { primaryKey: true })]);

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
    it("takes the entity's own read options, not the generic ones", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub struct InvoiceReadOptions {");
        expect(code).toContain("pub select: Vec<InvoiceSelectField>,");
        expect(code).toContain("pub filters: Vec<InvoiceFilter>,");
        expect(code).toContain("pub order: Vec<InvoiceOrder>,");
        expect(code).toContain("pub fn build_query_invoice(options: InvoiceReadOptions) -> Result<PlannedQuery, ReadError>");
        expect(code).toContain("pub async fn query_invoice(");
        expect(code).toContain("    options: InvoiceReadOptions,");
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
        expect(code).toContain("pub fn filter_and(");
        expect(code).toContain("pub fn filter_or(");
        expect(code).toContain("pub fn order(mut self, order: InvoiceOrder) -> Self");
    });

    it("accepts the same column twice, since the list is combined with and", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("self.filters.push(filter.into());");
        expect(code).toContain("The same column may be filtered twice.");
        expect(code).toContain("pub filters: Vec<InvoiceFilter>,");
    });

    it("gives each filterable column a newtype over its own comparison", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub struct InvoiceTenantIdFilter(pub Comparison<TenantId>);");
        expect(code).toContain("#[serde(transparent)]");
        expect(code).not.toContain("pub tenant_id: Option<");
    });

    it("names each column's type when it lowers to the planner", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain('TypedFilter::comparison("tenantId", SqlType::Uuid, self.0)');
        expect(code).toContain('TypedFilter::comparison("deletedAt", SqlType::Timestamptz, self.0)');
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

    it("flattens an and group so its children stay separate conditions", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("Self::And(filters) => Self::all(Combinator::And, filters),");
        expect(code).toContain("Self::Or(filters) => Self::all(Combinator::Or, filters),");
        expect(code).toContain("(Combinator::And, _) => typed,");
        expect(code).toContain("_ => vec![TypedFilter::group(combinator, typed)],");
    });

    it("serializes each field enum as its column name", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("#[derive(");
        expect(code).toContain("Clone, Copy, Debug, serde::Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize,");
        expect(code).toContain('    #[serde(rename = "tenantId")]\n    TenantId,');
    });

    it("serializes the read options as camelCase JSON, rejecting an unknown key", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain(
            "#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq, serde::Serialize)]",
        );
        expect(code).toContain('#[serde(rename_all = "camelCase", deny_unknown_fields)]\npub struct InvoiceReadOptions {');
        expect(code).toContain('#[serde(default, skip_serializing_if = "Vec::is_empty")]\n    pub select: Vec<InvoiceSelectField>,');
        expect(code).toContain('#[serde(default, skip_serializing_if = "Option::is_none")]\n    pub limit: Option<u64>,');
    });

    it("serializes an ordering clause with a defaulted direction", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain('#[serde(rename_all = "camelCase", deny_unknown_fields)]\npub struct InvoiceOrder {');
        expect(code).toContain("    #[serde(default)]\n    direction: SortDirection,");
    });

    it("decodes into the entity's selected shape", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub async fn query_invoice(");
        expect(code).toContain(") -> Result<Vec<InvoiceSelect>, ReadError> {");
        expect(code).toContain("decode_rows(rows)");
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
