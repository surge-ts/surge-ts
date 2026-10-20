// @strict: true
// `Array.isArray(x): x is any[]` keeps an array member as written, so its
// elements are still numbers: the expression narrowing and the module's own
// `if` both read `number[]`, not `any[]`.
export function inBody(j3: string | number[], j4: string | number[] | null) {
    const u1 = Array.isArray(j3) ? j3[0].toUpperCase() : "";
    if (Array.isArray(j4)) { j4[0].toUpperCase(); }
    const u3 = Array.isArray(j4) ? j4[0].toUpperCase() : "";
    return [u1, u3];
}
declare const j5: string | number[];
if (Array.isArray(j5)) { j5[0].toUpperCase(); }
export const u5 = Array.isArray(j5) ? j5[0].toUpperCase() : "";
// An `any[]` subject still takes a narrower predicate.
declare function isNumbers(value: unknown): value is number[];
declare const loose: any[];
if (isNumbers(loose)) { loose[0].toUpperCase(); }
