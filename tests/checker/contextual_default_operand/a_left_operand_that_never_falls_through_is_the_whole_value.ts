// `a || b` with a left that can never be falsy is `a` (`checkBinaryLikeExpression`
// under strictNullChecks): the right operand takes the contextual type but
// relates to nothing.
declare const o: { a: number };
const kept: { a: number } = o || {};
declare const maybe: { a: number } | undefined;
const fallback: { a: number } = maybe || {};
declare const text: string;
const either: number = text || 1;
const coalesced: { a: number } = maybe ?? {};
declare const flag: boolean;
const both: { a: number } = flag && {};
