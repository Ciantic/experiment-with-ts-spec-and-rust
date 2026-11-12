/** Unit tests for the filter-map generator, driven by self-contained fixtures. See docs/testing.md. */
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SPEC_SRC_ROOT, parseInMemorySpec, type SpecModel } from "../spec/spec-model.ts";
import { buildFilterModel, filterTypeName, renderFiltersModule, run } from "./generate-filters.ts";

/** The generator's two channels with nothing behind them, so an assertion reads the exit code. */
const SILENT = { log: () => undefined, error: () => undefined };

/** The glob that matches an in-memory fixture, placed so its import specifier looks like a real one. */
const SPEC_GLOB = join(SPEC_SRC_ROOT, "fixtures/**/*.ts");

/** An entity whose columns whitelist several operators, and whose key is a filter with no tag. */
const WIDGET = `
/**
 * @pgTable widget
 * @queries query
 * @entity
 */
export interface Widget {
    /** @primaryKey */
    id: string;

    /** @queryFilter ops=eq,any */
    tenantId: string;

    /** @queryFilter ops=gte,lte,between */
    startsAt?: Date;

    /** @queryFilter ops=lte,gte */
    endsAt?: Date;

    notes?: string;
}
`.trim();

/** A second read entity, so name order is observable. */
const AUDIT = `
/**
 * @pgTable audit
 * @queries query
 * @entity
 */
export interface Audit {
    /** @primaryKey */
    id: string;
}
`.trim();

/** An entity that declares no read, so it has no map to share. */
const UNREAD = `
/**
 * @pgTable marker
 * @entity
 */
export interface Marker {
    /** @primaryKey */
    id: string;
}
`.trim();

/** Parse an in-memory fixture, so no test reads the real spec. */
function specOf(entities: Record<string, string>): SpecModel {
    const sources = Object.entries(entities).map(([name, sourceFileText]) => ({
        filePath: join(SPEC_SRC_ROOT, "fixtures", `${name}.ts`),
        sourceFileText,
    }));
    return parseInMemorySpec(sources, { sourceGlob: SPEC_GLOB });
}

describe("buildFilterModel", () => {
    it("names each filterable column with its operators, in declaration order", () => {
        const model = buildFilterModel(specOf({ Widget: WIDGET }));

        expect(model).toEqual([
            {
                entity: "Widget",
                typeName: "WidgetFilter",
                columns: [
                    { name: "id", operators: ["eq"] },
                    { name: "tenantId", operators: ["eq", "any"] },
                    { name: "startsAt", operators: ["gte", "lte", "between"] },
                    { name: "endsAt", operators: ["gte", "lte"] },
                ],
            },
        ]);
    });

    it("keeps the operator order the vocabulary gives, whatever the tag wrote", () => {
        const model = buildFilterModel(specOf({ Widget: WIDGET }));
        const endsAt = model[0]?.columns.find((column) => column.name === "endsAt");

        expect(endsAt?.operators).toEqual(["gte", "lte"]);
    });

    it("leaves out a column that carries no @queryFilter", () => {
        const model = buildFilterModel(specOf({ Widget: WIDGET }));

        expect(model[0]?.columns.map((column) => column.name)).not.toContain("notes");
    });

    it("leaves out an entity that declares no read", () => {
        const model = buildFilterModel(specOf({ Widget: WIDGET, Marker: UNREAD }));

        expect(model.map((entry) => entry.entity)).toEqual(["Widget"]);
    });

    it("is stable in name order", () => {
        const model = buildFilterModel(specOf({ Widget: WIDGET, Audit: AUDIT }));

        expect(model.map((entry) => entry.typeName)).toEqual(["AuditFilter", "WidgetFilter"]);
    });
});

describe("renderFiltersModule", () => {
    it("writes one operator map per entity", () => {
        const code = renderFiltersModule(buildFilterModel(specOf({ Widget: WIDGET })));

        expect(code).toContain("export type WidgetFilter = {");
        expect(code).toContain('    id: "eq";');
        expect(code).toContain('    tenantId: "eq" | "any";');
        expect(code).toContain('    startsAt: "gte" | "lte" | "between";');
        expect(code).toContain("};");
    });

    it("quotes a key that is not a plain identifier", () => {
        const code = renderFiltersModule([
            { entity: "Widget", typeName: "WidgetFilter", columns: [{ name: "tenant-id", operators: ["eq"] }] },
        ]);

        expect(code).toContain('    "tenant-id": "eq";');
    });

    it("names the type after the entity", () => {
        expect(filterTypeName("Widget")).toBe("WidgetFilter");
    });
});

describe("run", () => {
    const created: string[] = [];

    afterEach(() => {
        for (const directory of created.splice(0)) {
            rmSync(directory, { recursive: true, force: true });
        }
    });

    it("writes the module to the file it is given", () => {
        const directory = mkdtempSync(join(tmpdir(), "filters-"));
        created.push(directory);
        const file = join(directory, "filters.ts");

        const code = run(specOf({ Widget: WIDGET }), ["--out", file], SILENT);

        expect(code).toBe(0);
        expect(readFileSync(file, "utf8")).toContain("export type WidgetFilter = {");
    });

    it("rejects --out without a file", () => {
        expect(run(specOf({ Widget: WIDGET }), ["--out"], SILENT)).toBe(1);
    });
});
