/** Unit tests for the Rust model generator, driven by self-contained fixtures. See docs/testing.md. */
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SPEC_SRC_ROOT, parseInMemorySpec } from "../spec/spec-model.ts";
import { buildRustModels, snakeName } from "./generate-models.ts";

/** The glob that matches an in-memory fixture, placed so its import specifier looks like a real one. */
const SPEC_GLOB = join(SPEC_SRC_ROOT, "**/*.ts");

/** A generic identifier primitive whose default a field inherits, which relaxes the create. */
const BRANDED_ID = `
/**
 * @primitive
 * @pgType uuid
 * @pgDefault uuidv7()
 */
export type BrandedId<Name extends string> = string;
`.trim();

/** A served value primitive, referenced from an entity in another directory. */
const ADDRESS = `
/**
 * @primitive
 * @pgType text
 */
export type Address = string & Brand<"Address">;
`.trim();

/** A child entity, so the parent has a branch to select and to leave out of a create. */
const WIDGET_ROW = `
import type { BrandedId } from "../primitives/BrandedId.ts";

/** The unique identifier for a widget row. */
export type WidgetRowId = BrandedId<"WidgetRowId">;

/**
 * @entity
 */
export interface WidgetRow {
    /** @primaryKey */
    id: WidgetRowId;

    description?: string;
}
`.trim();

/** An entity carrying a key alias, a branch, a clock, a version, and a type from another module. */
const WIDGET = `
import type { Address } from "../primitives/Address.ts";
import type { BrandedId } from "../primitives/BrandedId.ts";
import type { WidgetRow } from "./WidgetRow.ts";

/** The unique identifier for a widget. */
export type WidgetId = BrandedId<"WidgetId">;

/**
 * @entity
 */
export interface Widget {
    /** @primaryKey */
    id: WidgetId;

    name: string;

    notes?: string;

    tags?: string[];

    address?: Address;

    /** @children */
    rows?: WidgetRow[];

    /** @createdAt */
    createdAt: Date;

    /**
     * @version
     * @pgDefault 0
     */
    version: bigint;
}
`.trim();

/** The fixture model, with the spec directories the output has to mirror. */
function model() {
    const files = [
        { filePath: join(SPEC_SRC_ROOT, "primitives/BrandedId.ts"), sourceFileText: BRANDED_ID },
        { filePath: join(SPEC_SRC_ROOT, "primitives/Address.ts"), sourceFileText: ADDRESS },
        { filePath: join(SPEC_SRC_ROOT, "widget-service/Widget.ts"), sourceFileText: WIDGET },
        { filePath: join(SPEC_SRC_ROOT, "widget-service/WidgetRow.ts"), sourceFileText: WIDGET_ROW },
    ];
    return parseInMemorySpec(files, { sourceGlob: SPEC_GLOB });
}

/** The generated module of the `Widget` fixture. */
function widgetSource(): string {
    return buildRustModels(model()).files.get("widget-service/widget.rs") ?? "";
}

/** One struct of the generated module, from its declaration to the next item's doc comment. */
function structOf(source: string, name: string): string {
    const start = source.indexOf(`pub struct ${name} {`);
    if (start === -1) {
        return "";
    }
    const end = source.indexOf("\n}", start);
    return source.slice(start, end === -1 ? undefined : end);
}

describe("snakeName", () => {
    it("splits camel case and acronyms", () => {
        expect(snakeName("EInvoiceAddress")).toBe("e_invoice_address");
        expect(snakeName("createdAt")).toBe("created_at");
        expect(snakeName("UUIDv7")).toBe("uuidv7");
    });
});

describe("buildRustModels", () => {
    it("mirrors the spec directories and names modules in snake case", () => {
        const { files } = buildRustModels(model());

        expect([...files.keys()].sort()).toEqual([
            "mod.rs",
            "primitives/address.rs",
            "primitives/mod.rs",
            "widget-service/mod.rs",
            "widget-service/widget.rs",
            "widget-service/widget_row.rs",
        ]);
        expect(files.get("mod.rs")).toContain('#[path = "widget-service/mod.rs"] pub mod widget_service;');
        expect(files.get("widget-service/mod.rs")).toContain("pub mod widget;");
    });

    it("renders a struct with camel-cased fields and optional fields as Option", () => {
        const source = widgetSource();

        expect(source).toContain('#[serde(rename_all = "camelCase")]');
        expect(source).toContain("pub struct Widget {");
        expect(source).toContain("pub name: String,");
        expect(source).toContain('#[serde(default, skip_serializing_if = "Option::is_none")]\n    pub notes: Option<String>,');
        expect(source).toContain("pub tags: Option<Vec<String>>,");
        expect(source).toContain("pub address: Option<Address>,");
        expect(source).toContain("pub struct WidgetId(pub String);");
    });

    it("imports a type declared in another spec directory", () => {
        expect(widgetSource()).toContain("use crate::domain::primitives::address::Address;");
    });

    it("does not generate a generic primitive the spec only uses as a base", () => {
        const { files } = buildRustModels(model());

        expect(files.has("primitives/branded_id.rs")).toBe(false);
    });
});

describe("the selected shape", () => {
    it("tracks presence on every field and nests a branch's own selection", () => {
        const source = widgetSource();

        expect(source).toContain("pub struct WidgetSelect {");
        expect(source).toContain("pub id: Partial<WidgetId>,");
        expect(source).toContain("pub notes: Partial<Option<String>>,");
        expect(source).toContain("pub rows: Partial<Option<Vec<WidgetRowSelect>>>,");
    });

    it("imports the presence type and a branch's selection", () => {
        const source = widgetSource();

        expect(source).toContain("use crate::Partial;");
        expect(source).toContain("use crate::domain::widget_service::widget_row::WidgetRowSelect;");
    });
});

describe("the create shape", () => {
    it("leaves out a branch and a key the database assigns", () => {
        const insert = structOf(widgetSource(), "WidgetInsert");

        expect(insert).not.toContain("pub rows:");
        expect(insert).toContain("pub id: Option<WidgetId>,");
    });

    it("relaxes a clock and a defaulted version, keeping a plain field required", () => {
        const insert = structOf(widgetSource(), "WidgetInsert");

        expect(insert).toContain("pub name: String,");
        expect(insert).toContain("pub created_at: Option<chrono::DateTime<chrono::Utc>>,");
        expect(insert).toContain("pub version: Option<i64>,");
    });

    it("rejects a key a create would ignore", () => {
        const source = widgetSource();
        const declaration = source.slice(0, source.indexOf("pub struct WidgetInsert {"));

        expect(declaration).toContain('#[serde(rename_all = "camelCase", deny_unknown_fields)]');
    });
});

describe("the upsert shape", () => {
    it("claims the key and the version a create may leave to the database", () => {
        const upsert = structOf(widgetSource(), "WidgetUpsert");

        expect(upsert).toContain("pub id: WidgetId,");
        expect(upsert).toContain("pub version: i64,");
        expect(upsert).toContain("pub created_at: Option<chrono::DateTime<chrono::Utc>>,");
    });
});

describe("the patch shape", () => {
    it("locks the key and version, and tracks presence on everything else", () => {
        const patch = structOf(widgetSource(), "WidgetPatch");

        expect(patch).toContain("pub id: WidgetId,");
        expect(patch).toContain("pub version: i64,");
        expect(patch).toContain("pub name: Partial<String>,");
        expect(patch).toContain("pub notes: Partial<Option<String>>,");
    });

    it("leaves out a creation clock and a branch", () => {
        const patch = structOf(widgetSource(), "WidgetPatch");

        expect(patch).not.toContain("created_at");
        expect(patch).not.toContain("pub rows:");
    });
});

describe("the key shape", () => {
    it("names every key column and nothing else", () => {
        const key = structOf(widgetSource(), "WidgetPrimaryKey");

        expect(key).toContain("pub id: WidgetId,");
        expect(key).not.toContain("pub name:");
        expect(key).not.toContain("pub version:");
    });
});
