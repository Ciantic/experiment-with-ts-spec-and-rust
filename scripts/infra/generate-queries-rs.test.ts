/** Unit tests for the Rust query generator, driven by self-contained fixtures. See docs/testing.md. */
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SPEC_SRC_ROOT, parseInMemorySpec } from "../spec/spec-model.ts";
import type { Column, Table } from "./postgres-model.ts";
import {
    generateQueryModules,
    queryFileName,
    queryFunctionName,
    filterFields,
    renderFilters,
    renderOrder,
    renderQueryModule,
    renderSelection,
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

describe("renderQueryModule", () => {
    it("names the entity's own query type, and leaves the builder to the shared one", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("pub type InvoiceQuery = EntityQuery<Invoice>;");
        expect(code).toContain("impl ReadEntity for Invoice {");
        expect(code).toContain("    type Selection = InvoiceSelection;");
        expect(code).toContain("    type Filter = InvoiceFilter;");
        expect(code).toContain("    type Order = InvoiceOrder;");
        // The fluent builder is written once in infra, so no module emits one.
        expect(code).not.toContain("pub fn filter(");
        expect(code).not.toContain("pub fn as_sql(");
        expect(code).not.toContain("pub async fn query(");
    });

    it("lowers the typed selection, filters, and ordering to the builder's own types", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("    fn lower(");
        expect(code).toContain('    const TABLE: &\'static str = "invoice";');
        expect(code).toContain("        Ok(LoweredRead(ReadOptions {");
        expect(code).toContain("columns: select.0.into_iter().map(|field| field.select()).collect(),");
        expect(code).toContain("typed.extend(filter.into_typed_filters()?);");
        expect(code).toContain("sort: order.into_iter().map(|clause| clause.into_sort()).collect(),");
    });

    it("builds a group through the entity's own filter, so the kind reaches the wire", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("fn group(kind: FilterKind, filters: Vec<Self::Filter>) -> Self::Filter {");
        expect(code).toContain("InvoiceFilter::Group { kind, filters }");
    });

    it("hands the selection to the shared macro", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("query_selection! {\n    Invoice {");
        expect(code).toContain("use macro_helpers::{query_filters, query_order, query_selection};");
        expect(code).not.toContain("pub enum InvoiceSelectField {");
        expect(code).not.toContain("pub struct InvoiceSelection(");
    });

    it("hands each column's wire shape and the entity's filter to the shared macro", () => {
        const code = renderQueryModule(invoice, spec);

        // The macro writes each column's enum, the umbrella enum over them, and every lowering.
        expect(code).toContain("query_filters! {\n    Invoice {");
        expect(code).toContain("use macro_helpers::{query_filters, query_order, query_selection};");
        expect(code).not.toContain("flat_filter::serialize(");
        expect(code).not.toContain("pub enum InvoiceFilter {");
        expect(code).not.toContain("impl From<InvoiceTenantIdFilter> for InvoiceFilter {");
    });

    it("lets the builder group through the entity's own filter", () => {
        const code = renderQueryModule(invoice, spec);

        // The builder writes the group, and the macro writes the variant it names.
        expect(code).toContain("InvoiceFilter::Group { kind, filters }");
        expect(code).toContain("fn group(kind: FilterKind, filters: Vec<Self::Filter>) -> Self::Filter {");
    });

    it("hands the ordering clause to the shared macro", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("query_order! {\n    Invoice {");
        expect(code).not.toContain("pub struct InvoiceOrder {");
        expect(code).not.toContain("serialize_tuple(2)");
    });

    it("gives the macro each column's value type, SQL type, and ops", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).toContain("tenantId: TenantId, Uuid, [eq],");
        expect(code).toContain("id: InvoiceId, Uuid, [eq],");
        expect(code).toContain("deletedAt: chrono::DateTime<chrono::Utc>, Timestamptz, [eq],");
        expect(code).toContain(
            "issueDate: chrono::DateTime<chrono::Utc>, Timestamptz, [gte, lte, between],",
        );
        // Only the ops the tag whitelists, so a column never offers a comparison the spec forbids.
        expect(code).not.toContain("issueDate: chrono::DateTime<chrono::Utc>, Timestamptz, [gte, gt,");
    });

    it("writes one invocation per field set, with its columns in declaration order", () => {
        expect(renderSelection("Invoice", ["id", "tenantId"])).toEqual([
            "// Writes `InvoiceSelectField` and `InvoiceSelection`.",
            "query_selection! {",
            "    Invoice {",
            "        id,",
            "        tenantId,",
            "    }",
            "}",
        ]);
        expect(renderOrder("Invoice", ["number", "updatedAt"])).toEqual([
            "// Writes `InvoiceOrderField` and `InvoiceOrder`.",
            "query_order! {",
            "    Invoice {",
            "        number,",
            "        updatedAt,",
            "    }",
            "}",
        ]);
    });

    it("names the types each invocation writes, wrapping when the list is long", () => {
        const code = renderQueryModule(invoice, spec);
        const filters = renderFilters("Invoice", filterFields(invoice, spec));

        expect(code).toContain("// Writes `InvoiceSelectField` and `InvoiceSelection`.");
        expect(code).toContain("// Writes `InvoiceOrderField` and `InvoiceOrder`.");
        const comment = filters.filter((line) => line.startsWith("//"));
        expect(comment.join(" ")).toContain("`InvoiceIssueDateFilter`, and `InvoiceFilter`.");
        // A filled line never runs past the file's width, and every line of the list is a comment.
        expect(comment.every((line) => line.length <= 100)).toBe(true);
    });

    it("keeps the read on the builder, with no free function beside it", () => {
        const code = renderQueryModule(invoice, spec);

        expect(code).not.toContain("pub async fn query_invoice(");
        expect(code).not.toContain("pub fn build_query_invoice(");
    });

    it("names only the columns each capability marks", () => {
        const code = renderQueryModule(invoice, spec);
        const selection = code.slice(code.indexOf("query_selection!"), code.indexOf("query_filters!"));
        const order = code.slice(code.indexOf("query_order!"), code.indexOf("pub type InvoiceQuery"));

        // An inlined column belongs to its branch, so a read does not select it by name.
        expect(selection).not.toContain("customerName");
        // Only a `@queryOrderBy` column may order, and `number` is the one this fixture marks.
        expect(order).toContain("        number,");
        expect(order).not.toContain("customerId");
    });

    it("writes an empty field set when the entity declares none", () => {
        expect(renderOrder("AuditLog", [])).toEqual([
            "// Writes `AuditLogOrderField` and `AuditLogOrder`.",
            "query_order! { AuditLog {} }",
        ]);
    });

    it("lets a nullable column offer a null test, which binds nothing", () => {
        const code = renderQueryModule(nullable, spec);
        const invocation = code.slice(code.indexOf("query_filters!"), code.indexOf("/// A filter over"));

        expect(invocation).toContain("consumedAt:");
        expect(invocation).toContain("[isNull, isNotNull],");
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
