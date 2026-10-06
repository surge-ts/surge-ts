// @module: commonjs
// @filename: asserts.ts
function isNonNullable<T>(obj: T): asserts obj is NonNullable<T> {
    if (obj === undefined || obj === null) throw new Error("nullable");
}
export { isNonNullable };
// @filename: test.ts
import * as asserts from "./asserts";
function test(obj: string | null): void {
    asserts.isNonNullable(obj);
    obj.trim();
}
function untouched(obj: string | null): void {
    obj.trim();
}
