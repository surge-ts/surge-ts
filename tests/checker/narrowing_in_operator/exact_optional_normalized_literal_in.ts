// @exactOptionalPropertyTypes: true
// A normalized object literal union gains its siblings' properties as tsc's
// missing type, which `"p" in o` removes from a read of `o.p`.
export function main(a: string[] | undefined) {
    const z = a ? { a } : { b: ["there"] };
    z.a ? z.a.toString() : z.b.toString();
    "a" in z ? z.a.toString() : z.b.toString();
    const n: number = z.a;
    if ("a" in z) {
        const m: number = z.a;
    }
}
