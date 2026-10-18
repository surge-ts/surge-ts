// Destructuring a call whose result is a union of branded tuples (`LuaTuple<[A,
// B] | [C, D]>`, each member a tuple intersected with an object) reads each
// member's element, as destructuring a variable of that type does.
type LuaTuple<T extends Array<any>> = T & { readonly LUA_TUPLE: never };
declare function result(): LuaTuple<[true, string] | [false, number]>;
declare const held: LuaTuple<[true, string] | [false, number]>;
export function f() {
	const [ok, value] = result();
	const flag: number = ok;
	const read: boolean = value;
	const [heldOk] = held;
	const heldFlag: number = heldOk;
}
