// A type query's name is checked as an expression: a qualifier that may be
// `null` or `undefined` is reported, by name when it is the leading one.
declare const maybe: { a: { b: number } | null } | undefined;
let first: typeof maybe.a;
let deeper: typeof maybe.a.b;
declare const nullable: { a: number } | null;
let named: typeof nullable.a;
function method(this: { no: number } | undefined) {
    let fromThis: typeof this.no = 1;
    return fromThis;
}
declare const both: { a: number } | null | undefined;
let either: typeof both.a;
declare const present: { a: number } | undefined;
if (present) {
    let narrowed: typeof present.a = 1;
}
export { first, deeper, named, method, either };
