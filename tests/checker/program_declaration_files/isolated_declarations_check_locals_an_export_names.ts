// @isolatedDeclarations: true
// @declaration: true
// @strict: true
// A local named by a type query or a computed key of an exported declaration
// is emitted too, so it needs an annotation of its own.
let keyed = Symbol();
export let literal = { [keyed]: 1 };
let queried = Symbol();
export let annotated: typeof queried;
let interfaceKey = Symbol();
export interface Keyed { [interfaceKey]: 1 }
let parameterType = 1 + 1;
export function takes(value: typeof parameterType): void {}
const holder = { key: Symbol() };
export let nested = { [holder.key]: 1 };
let chained = Symbol();
let viaChain: typeof chained = chained;
export let reached: typeof viaChain;
let valueOnly = Symbol();
export const plain = { value: valueOnly };
let bodyOnly = Symbol();
export function body(): void {
    const inner = { [bodyOnly]: 1 };
}
let unused = Symbol();
