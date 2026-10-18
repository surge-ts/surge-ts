// A generic global declared in another file, by a script or a `declare
// global` block, infers its type arguments from the call like a local does.
// @filename: globals.d.ts
declare const scriptConst: { <T>(value: T): { got: T } };
declare function scriptFunction<T>(value: T): { got: T };
declare const scriptArrow: <T>(value: T) => { got: T };

// @filename: augment.d.ts
export {};
declare global {
	const augmentedConst: { <T>(value: T): { got: T }; extend(matchers: object): void };
	function augmentedFunction<T>(value: T): { got: T };
}

// @filename: main.ts
export {};
const a = scriptConst(1);
const b = scriptFunction(1);
const c = scriptArrow(1);
const d = augmentedConst(1);
const e = augmentedFunction(1);
const sa: string = a.got;
const sb: string = b.got;
const sc: string = c.got;
const sd: string = d.got;
const se: string = e.got;
const direct: string = augmentedConst(1).got;
