// @strict: true
// An intersection over a union of tuples is a union of intersections, and a
// literal index reads each member as tsc's `createUnionOrIntersectionProperty`
// does: a member with the element answers with it, and one without answers
// through its number index, which is `never` for `[]` and the elements of an
// open tuple. roblox-ts's `LuaTuple<T> = T & { readonly LUA_TUPLE: never }`
// over what `string.find` returns.
export {};
type LuaTuple<T extends Array<any>> = T & { readonly LUA_TUPLE: never };

declare function maybe(): LuaTuple<[] | [number, string]>;
const first: number = maybe()[0];
const firstNot: string = maybe()[0];
const second: string = maybe()[1];
const past: string | number = maybe()[2];
const pastNot: number = maybe()[2];
const [destructured] = maybe();
const destructuredRead: number = destructured;
declare const held: LuaTuple<[] | [number, string]>;
const [heldFirst] = held;
const heldFirstRead: number = heldFirst;

declare function fixed(): LuaTuple<[number] | [string, boolean]>;
const fixedSecond: number | boolean = fixed()[1];
const fixedSecondNot: boolean = fixed()[1];

declare function find(): LuaTuple<[number, number, ...Array<string | number>] | Array<undefined>>;
const found = find();
const start: number | undefined = found[0];
const startNot: number = found[0];
const capture: string | number | undefined = found[2];
const captureNot: string | number = found[2];
if (found[0]) {
	const narrowed: number = found[0];
}
const [foundStart, foundEnd] = find();
const foundStartRead: number = foundStart;
const foundEndRead: number | undefined = foundEnd;

declare function open(): LuaTuple<[number, ...string[]]>;
const head: number = open()[0];
const tail: string = open()[1];
const size: number = open().length;
const [openHead, openTail] = open();
const openHeadRead: string = openHead;
const openTailRead: number = openTail;

declare function empty(): LuaTuple<[]>;
const nothing: never = empty()[0];

type Indexed = LuaTuple<[] | [number, string]>[0];
const indexed: Indexed = "x";
type Returned = ReturnType<typeof find>[0];
const returned: Returned = "x";

declare const plain: [] | [number, string];
const plainFirst: number = plain[0];
declare const openOrFixed: [number, ...string[]] | [boolean];
const openOrFixedTail: string = openOrFixed[1];
