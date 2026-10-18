// `chooseOverload` checks a callback that needs no context (no parameter or
// every one annotated) as it is, so a candidate whose callback slot returns
// what the callback cannot is passed over for a later one.
type Branded<T extends Array<any>> = T & { readonly BRAND: never };
declare function pick<U extends Array<any>>(callback: () => Branded<U>): { branded: U };
declare function pick<U>(callback: () => U): { plain: U };
declare function strings(callback: () => string[]): { strings: true };
declare function strings<U>(callback: () => U): { other: U };
declare function annotated(callback: (value: number) => string): { text: string };
declare function annotated(callback: (value: number) => number): { count: number };
const plain: number = pick(() => 1);
const other: number = strings(() => 1);
const listed: number = strings(() => ["a"]);
const counted: string = annotated((value: number) => value);
const texted: number = annotated((value: number) => String(value));
export {};
