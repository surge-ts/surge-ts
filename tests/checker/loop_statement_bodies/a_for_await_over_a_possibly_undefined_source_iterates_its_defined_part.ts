// @strict: true
// `checkNonNullExpression` reports a possibly-`undefined` source and the
// loop iterates its defined part, so the binding is still the element type.
export async function drain(source: AsyncIterable<number> | undefined) {
    for await (const x of source) {
        const s: string = x;
    }
}
