/** Type-level tests for the selection projection. See docs/queries.md. */
import { describe, expect, it } from "vitest";
import type { Filters, Selected } from "./selection.ts";

/** A stand-in entity: a required scalar, a nullable one, and an optional branch. */
interface Widget {
    id: string;
    code?: string;
    rows?: { note: string; hidden?: string }[];
}

describe("Selected", () => {
    /** Never called: each annotation is the assertion, checked by `tsc --noEmit`. */
    function assertTypes(): void {
        const required: Selected<Widget, { id: true }> = { id: "w" };
        // @ts-expect-error a selected required field stays required
        const missing: Selected<Widget, { id: true }> = {};
        const optional: Selected<Widget, { code: true }> = {};
        const branch: Selected<Widget, { rows: { select: { note: true } } }> = {
            rows: [{ note: "n" }],
        };
        const wrongBranch: Selected<Widget, { rows: { select: { note: true } } }> = {
            // @ts-expect-error a field the branch does not select is absent
            rows: [{ note: "n", hidden: "h" }],
        };

        void [required, missing, optional, branch, wrongBranch];
    }

    it("keeps a selected field's optionality", () => {
        expect(assertTypes).toBeTypeOf("function");
    });
});

describe("Filters", () => {
    /** The entity the filter cases read, with a nullable column so a null test is offered. */
    interface Filtered {
        id: string;
        code?: string;
        deletedAt?: Date;
    }

    /** The generated filter type: one flat comparison per column and operator. */
    type FilteredFilter =
        | { id__eq: string }
        | { id__any: readonly string[] }
        | { code__eq: string }
        | { deletedAt__eq: Date }
        | { deletedAt__isNull: true }
        | { deletedAt__isNotNull: true };

    /** Never called: each annotation is the assertion, checked by `tsc --noEmit`. */
    function assertTypes(): void {
        const leaf: Filters<FilteredFilter> = [{ id__eq: "w" }, { deletedAt__isNull: true }];
        const group: Filters<FilteredFilter> = [
            { id__eq: "w" },
            { kind: "or", filters: [{ code__eq: "c" }, { deletedAt__isNotNull: true }] },
        ];
        const wrongColumn: Filters<FilteredFilter> = [
            // @ts-expect-error a column the generated type does not name is not filterable
            { note__eq: "n" },
        ];
        const wrongOperator: Filters<FilteredFilter> = [
            // @ts-expect-error the column's ops= does not offer gt
            { id__gt: "w" },
        ];
        const wrongValue: Filters<FilteredFilter> = [
            // @ts-expect-error `any` takes a list, not one value
            { id__any: "w" },
        ];
        const wrongKind: Filters<FilteredFilter> = [
            // @ts-expect-error the only group a read accepts is `or`
            { kind: "and", filters: [{ id__eq: "w" }] },
        ];
        const nested: Filters<FilteredFilter> = [
            {
                kind: "or",
                // @ts-expect-error an or-group holds comparisons, never another group
                filters: [{ kind: "or", filters: [{ id__eq: "w" }] }],
            },
        ];

        void [leaf, group, wrongColumn, wrongOperator, wrongValue, wrongKind, nested];
    }

    it("takes flat comparisons and or groups over the generated filter type", () => {
        expect(assertTypes).toBeTypeOf("function");
    });
});
