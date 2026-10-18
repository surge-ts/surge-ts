// An intersection with a tuple operand reads, indexes and iterates as the
// tuple does: roblox-ts's `LuaTuple<T> = T & { readonly LUA_TUPLE: never }`.
export {};
type LuaTuple<T extends Array<any>> = T & { readonly LUA_TUPLE: never };
declare function $tuple<T extends Array<any>>(...values: T): LuaTuple<T>;

declare const written: [number, string] & { readonly tag: never };
const first: string = written[0];
const second: number = written[1];

const made = $tuple(1, "a");
const madeFirst: string = made[0];
const [destructured] = made;
const destructuredFirst: string = destructured;
declare const index: number;
const byNumber: boolean = made[index];
const length: string = made.length;
for (const element of made) {
	const each: boolean = element;
}
