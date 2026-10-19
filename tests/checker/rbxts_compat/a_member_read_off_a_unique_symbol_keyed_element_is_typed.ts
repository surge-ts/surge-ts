// An element read keyed by a unique symbol is the member declared under it,
// also where a property read continues from it (roblox-ts's
// `props[Roact.Children].name`).
declare const Key: unique symbol;
declare namespace Lib {
	const Children: unique symbol;
}
declare const bag: { [Key]: { foo: number } };
declare const props: { a: 1 } & { [Lib.Children]: { [name: string]: number } };
const direct: string = bag[Key].foo;
const wrapped: string = (bag[Key]).foo;
const nested: string = props[Lib.Children].anything;
export {};
