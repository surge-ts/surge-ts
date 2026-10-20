// @strict: true
// A declaration's type carries the predicate its body infers, so passing it
// where a predicate is expected infers the predicate's type parameter.
function isText(x: unknown) {
    return typeof x === "string";
}
declare function pick<T, S extends T>(xs: T[], p: (x: T) => x is S): S[];
declare const values: unknown[];
export const picked: number[] = pick(values, isText);
export function inner() {
    function isNum(x: unknown) {
        return typeof x === "number";
    }
    const nums: string[] = pick(values, isNum);
    return nums;
}
