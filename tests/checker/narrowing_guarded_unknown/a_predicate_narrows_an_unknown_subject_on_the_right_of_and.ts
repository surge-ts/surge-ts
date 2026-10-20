// @strict: true
// A predicate over an `unknown` binding narrows it for the right operand of
// `&&` too, so the result carries the predicate's type.
declare function isObject(v: unknown): v is Record<string, unknown>;
declare function isStr(v: unknown): v is string;
export function narrowed(err: unknown) {
    const asObject: number = isObject(err) && err;
    const asString: number = isStr(err) && err;
    return [asObject, asString];
}
