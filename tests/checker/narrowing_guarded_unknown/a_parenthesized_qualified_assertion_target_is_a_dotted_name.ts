// @strict: true
// `isDottedName` looks through parentheses: only a target that is not an
// identifier or qualified name at all is TS2776.
declare namespace Debug {
    function check(value: unknown): asserts value;
}
declare const checks: ((value: unknown) => asserts value)[];
export function parenthesized(x: unknown) {
    ((Debug).check)(x);
    (Debug.check)(x);
}
export function indexed(x: unknown) {
    checks[0](x);
}
