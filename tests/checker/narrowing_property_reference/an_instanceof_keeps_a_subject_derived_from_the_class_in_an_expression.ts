// @strict: true
// `instanceof` keeps a subject already derived from the class
// (`isTypeDerivedFrom` first), in a conditional expression as in an `if`.
export class ClosedError extends Error {
    code = 1;
}
export function narrowed(error: ClosedError | null | undefined, list: string[] | string) {
    const closed: ClosedError | 0 = error instanceof Error ? error : 0;
    const first = (list instanceof Array ? list : [])[0];
    first.toFixed();
    return closed;
}
