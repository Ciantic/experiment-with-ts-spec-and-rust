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
export type InvoiceId = string;
export type TenantId = string;

/**
 * @entity
 * @pgTable invoice
 * @queries query
 */
export interface Invoice {
    /** @primaryKey */
    id: InvoiceId;
    tenantId: TenantId;
    number?: string;
    updatedAt: Date;
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
        expect(selectableFields(invoice)).toEqual(["id", "tenantId", "number", "updatedAt"]);
    });
});

describe("fieldEnumName", () => {
    it("names one enum per capability", () => {
        expect(fieldEnumName("Invoice", "Select")).toBe("InvoiceSelectField");
        expect(fieldEnumName("Invoice", "Filter")).toBe("InvoiceFilterField");
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

        expect(code).toContain("pub(crate) fn into_read_options(self) -> ReadOptions {");
        expect(code).toContain(".map(|field| field.select())");
        expect(code).toContain(".map(|filter| filter.into_typed_filter())");
        expect(code).toContain(".map(|order| order.into_sort())");
        expect(code).toContain('read.plan("invoice", None)');
    });

    it("builds the selection, filters, and ordering with a small fluent API", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub fn with_select(fields: impl IntoIterator<Item = InvoiceSelectField>) -> Self");
        expect(code).toContain("pub fn filter(mut self, filter: InvoiceFilter) -> Self");
        expect(code).toContain("pub fn order(mut self, order: InvoiceOrder) -> Self");
    });

    it("names each filter's column type, so a driver can bind it", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub const fn sql_type(self) -> SqlType {");
        expect(code).toContain("Self::TenantId => SqlType::Uuid,");
        expect(code).toContain(
            "} => TypedFilter::condition(field.as_str(), field.sql_type(), operator, value),",
        );
    });

    it("takes each field's own model type, so a mismatched value cannot compile", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub const ID: InvoiceIdFilter = InvoiceIdFilter(Self::Id);");
        expect(code).toContain("pub const TENANT_ID: InvoiceTenantIdFilter = InvoiceTenantIdFilter(Self::TenantId);");
        expect(code).toContain("pub fn eq(self, value: impl Borrow<InvoiceId>) -> Result<InvoiceFilter, ReadError> {");
        expect(code).toContain("pub fn eq(self, value: impl Borrow<TenantId>) -> Result<InvoiceFilter, ReadError> {");
        expect(code).toContain("values: impl IntoIterator<Item = impl Borrow<TenantId>>,");
        expect(code).toContain("use models::domain::fixtures::TenantId;");
    });

    it("accepts a value or a borrow of it", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("use std::borrow::Borrow;");
        expect(code).toContain("value.borrow()");
        expect(code).toContain("value.borrow().to_bind_value()?");
    });

    it("leaves no route from a raw datagrid value to a filter", () => {
        const code = renderQueryModule(invoice, spec);

        // The constructor and the lowering method are the module's own, not a caller's.
        expect(code).toContain("    fn condition(");
        expect(code).not.toContain("    pub fn condition(");
        expect(code).toContain("pub(crate) fn into_read_options(self) -> ReadOptions {");
    });

    it("keeps a null test infallible, since it binds no value", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub fn is_null(self) -> InvoiceFilter {");
        expect(code).toContain("InvoiceFilter::condition(self.0, FilterOperator::IsNull, None)");
    });

    it("keeps a raw column name out of the caller's filters", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub enum InvoiceFilter {");
        expect(code).toContain("        field: InvoiceFilterField,");
        expect(code).not.toContain("    pub filters: Vec<TypedFilter>,");
        expect(code).not.toContain("pub fn filter(mut self, filter: TypedFilter) -> Self");
    });

    it("groups filters with and / or, each child keeping its column type", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub fn and(filters: impl IntoIterator<Item = Self>) -> Self");
        expect(code).toContain("pub fn or(filters: impl IntoIterator<Item = Self>) -> Self");
        expect(code).toContain("TypedFilter::group(Combinator::And, children)");
        expect(code).toContain("TypedFilter::group(Combinator::Or, children)");
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
        expect(code).toContain("pub enum InvoiceFilterField {");
        expect(code).toContain("pub enum InvoiceOrderField {");
        expect(code).toContain("Self::TenantId => \"tenantId\",");
    });

    it("builds filters, selects, and sorts from the typed fields", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub fn select(self) -> SelectColumn");
        expect(code).toContain("pub const fn field(self) -> InvoiceFilterField {");
        expect(code).toContain("pub fn asc(self) -> InvoiceOrder {");
        expect(code).toContain("pub struct InvoiceOrder {");
    });

    it("writes an empty field set when the entity declares none", () => {
        const code = renderQueryModule(bare, spec);

        expect(code).toContain("pub enum AuditLogFilterField {}");
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
