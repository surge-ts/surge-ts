declare function test(x: any, y: () => string): string | undefined;
declare function opt(x?: any, y?: any): void;
declare function rest(...xs: any[]): void;
declare function mixed(x: number, ...xs: any[]): void;
declare var anys: any[];
declare var anyv: any;
declare var ro: readonly any[];
test(...anys);
opt(...anys);
rest(...anys);
mixed(...anys);
test(...anyv);
test(1, ...anys);
test(...ro);
declare function g(a: number, b: string): void;
function viaGeneric<T extends any[]>(...args: T) { g(...args); }
function viaTupleConstraint<T extends [number, string]>(...args: T) { g(...args); }
function viaUnknown(...args: unknown[]) { g(...args); }
function viaTuple(...args: [number, string]) { g(...args); }
function forward<A extends unknown[]>(fn: (...a: A) => void, ...args: A) { fn(...args); }
export {};
