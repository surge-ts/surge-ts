// @strict: true
type Item = { value: string };
type ItemMap<T> = { [P in keyof T]: Item };
declare let x4: ItemMap<any>;
let n1: number = x4;
let n2: number = x4.foo;
type Arrayish<T extends unknown[]> = { [K in keyof T]: T[K] };
function bar(arrayish: Arrayish<any>) {
    let arr: any[];
    arr = arrayish;
}
declare function stringifyArray<T extends readonly any[]>(arr: T): { -readonly [K in keyof T]: string };
let abc: any[] = stringifyArray(void 0 as any);
declare function stringifyPair<T extends readonly [any, any]>(arr: T): { -readonly [K in keyof T]: string };
let def: [any, any] = stringifyPair(void 0 as any);
