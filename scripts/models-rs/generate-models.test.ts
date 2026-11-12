/** Unit tests for the Rust model generator, driven by self-contained fixtures. See docs/testing.md. */
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SPEC_SRC_ROOT, parseInMemorySpec } from "../spec/spec-model.ts";
import { buildRustModels, snakeName } from "./generate-models.ts";

/** The glob that matches an in-memory fixture, placed so its import specifier looks like a real one. */
const SPEC_GLOB = join(SPEC_SRC_ROOT, "**/*.ts");

/** A generic identifier primitive, the base of a domain alias. */
const BRANDED_ID = `
/**
 * @primitive
 * @pgType uuid
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

/** An entity carrying a key alias, an optional scalar, an optional array, and a type from another module. */
const WIDGET = `
import type { Address } from "../primitives/Address.ts";
import type { BrandedId } from "../primitives/BrandedId.ts";

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
}
`.trim();

/** The fixture model, with the spec directories the output has to mirror. */
function model() {
    const files = [
        { filePath: join(SPEC_SRC_ROOT, "primitives/BrandedId.ts"), sourceFileText: BRANDED_ID },
        { filePath: join(SPEC_SRC_ROOT, "primitives/Address.ts"), sourceFileText: ADDRESS },
        { filePath: join(SPEC_SRC_ROOT, "widget-service/Widget.ts"), sourceFileText: WIDGET },
    ];
    return parseInMemorySpec(files, { sourceGlob: SPEC_GLOB });
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
        ]);
        expect(files.get("mod.rs")).toContain('#[path = "widget-service/mod.rs"] pub mod widget_service;');
        expect(files.get("widget-service/mod.rs")).toContain("pub mod widget;");
    });

    it("renders a struct with camel-cased fields and optional fields as Option", () => {
        const { files } = buildRustModels(model());
        const source = files.get("widget-service/widget.rs") ?? "";

        expect(source).toContain('#[serde(rename_all = "camelCase")]');
        expect(source).toContain("pub struct Widget {");
        expect(source).toContain("pub name: String,");
        expect(source).toContain('#[serde(default, skip_serializing_if = "Option::is_none")]\n    pub notes: Option<String>,');
        expect(source).toContain("pub tags: Option<Vec<String>>,");
        expect(source).toContain("pub address: Option<Address>,");
        expect(source).toContain("pub struct WidgetId(pub String);");
    });

    it("imports a type declared in another spec directory", () => {
        const { files } = buildRustModels(model());
        const source = files.get("widget-service/widget.rs") ?? "";

        expect(source).toContain("use crate::domain::primitives::address::Address;");
    });

    it("does not generate a generic primitive the spec only uses as a base", () => {
        const { files } = buildRustModels(model());

        expect(files.has("primitives/branded_id.rs")).toBe(false);
    });
});
