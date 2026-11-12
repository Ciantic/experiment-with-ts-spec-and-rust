/** Read types: the column-limiting selection, the filter arguments, and the ordering clauses. See docs/queries.md. */
import type { Brand } from "spec/primitives/Brand.ts";

/** The value a filter matches when a read omits it: a literal, or a null test such as `isNull`. See docs/queries.md. */
export type FilterDefault =
    | { kind: "isNull" }
    | { kind: "isNotNull" }
    | { kind: "literal"; value: string | number | boolean };

/** A leaf column: a keyword, a branded primitive, a `Date`, or a union of those. */
type Scalar<T> = NonNullable<T> extends string | number | boolean | bigint | Date | Brand<any>
    ? true
    : false;

/** The element of a child collection, or the entity itself when it is not an array. */
type ElementOf<T> = NonNullable<T> extends readonly (infer U)[] ? NonNullable<U> : NonNullable<T>;

/** Whether `T` is a child collection. */
type IsArray<T> = NonNullable<T> extends readonly unknown[] ? true : false;

/** A sort direction. See docs/queries.md. */
export type Direction = "asc" | "desc";

/** One ordering clause of a read: `[field, direction]` over a whitelisted field. See docs/queries.md. */
export type Order<K extends PropertyKey> = [field: K, direction: Direction];

/** The operators that test whether a column is null; they take no value. See docs/queries.md. */
export type NullOp = "isNull" | "isNotNull";

/** The operators that compare a column with a list of values. See docs/queries.md. */
export type ListOp = "in" | "notIn" | "any" | "notAny";

/** The operators that bound a column from both sides, taking two values. See docs/queries.md. */
export type RangeOp = "between" | "notBetween";

/** The comparison operators a `@queryFilter ops=` may name. See docs/queries.md. */
export type CompareOp =
    | "eq"
    | "neq"
    | "gt"
    | "gte"
    | "lt"
    | "lte"
    | "like"
    | "ilike"
    | "notLike"
    | "notIlike"
    | ListOp
    | NullOp
    | RangeOp;

/** The value an operator takes: `true` for a null test, a pair for a range, a list for a set. See docs/queries.md. */
export type OperatorValue<P extends PropertyKey, V> = P extends NullOp
    ? true
    : P extends ListOp
      ? readonly V[]
      : P extends RangeOp
        ? readonly [V, V]
        : V;

/** The scalar field names of `E`: the fields a branch's `filters` or `order` may name. See docs/queries.md. */
export type ScalarKeys<E> = {
    [K in keyof E]-?: Scalar<E[K]> extends true ? K : never;
}[keyof E];

/** Each filterable field mapped to the operators its `@queryFilter ops=` names, as the generator writes them. See docs/queries.md. */
export type FilterOps<E> = Partial<Record<keyof E, PropertyKey>>;

/** One comparison on one column of `E`, restricted to that column's `@queryFilter ops=`. See docs/queries.md. */
export type Filter<E, O extends FilterOps<E>> = {
    [K in keyof O & keyof E]?: {
        [P in Extract<NonNullable<O[K]>, PropertyKey>]?: OperatorValue<P, NonNullable<E[K]>>;
    };
};

/** The filters a read must name first, one entry per required column. See docs/queries.md. */
type RequiredFilters<E, O extends FilterOps<E>, R extends readonly (keyof E)[]> = {
    [I in keyof R]: { [K in R[I] & keyof O]-?: NonNullable<Filter<E, O>[K]> };
};

/** The filters of a read, combined with `and`, so one column may carry more than one. `R` names the columns a read must filter. See docs/queries.md. */
export type Filters<
    E,
    O extends FilterOps<E>,
    R extends readonly (keyof E)[] = [],
> = R extends readonly [] ? readonly Filter<E, O>[] : readonly [...RequiredFilters<E, O, R>, ...Filter<E, O>[]];

/** The arguments of a read over `E`, as a branch carries them: the same shape a root read takes. See docs/queries.md. */
export type QueryArgs<E> = {
    select: Selection<E>;
    filters?: Filters<E, Partial<Record<keyof E, CompareOp>>>;
    order?: readonly (readonly [field: ScalarKeys<E>, direction: Direction])[];
    limit?: number;
    offset?: number;
};

/** The fields a caller wants: `true` for a scalar, `true` or a read's arguments for a branch. See docs/queries.md. */
export type Selection<E> = {
    [K in keyof E]?: Scalar<E[K]> extends true ? true : true | QueryArgs<ElementOf<E[K]>>;
};

/** The nested `select` a branch value carries, whether it was written as `true` or as read arguments. See docs/queries.md. */
type BranchSelect<B> = B extends { select: infer S } ? NonNullable<S> : never;

/** The value one selected key carries: the entity's field, or the selected shape of a branch. */
type SelectedValue<E, S, K extends keyof E & keyof S> = NonNullable<S[K]> extends true
    ? E[K]
    : IsArray<E[K]> extends true
        ? Selected<ElementOf<E[K]>, BranchSelect<S[K]>>[]
        : Selected<ElementOf<E[K]>, BranchSelect<S[K]>>;

/** The result of `E` under selection `S`: only the selected keys, each keeping the entity's optionality. See docs/queries.md. */
export type Selected<E, S> = {
    [K in keyof E as K extends keyof S ? K : never]: SelectedValue<E, S, K & keyof S>;
};
