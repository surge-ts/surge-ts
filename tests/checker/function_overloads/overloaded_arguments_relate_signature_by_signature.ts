interface P<T> { v: T }
declare function testFunction(n: number): P<number>;
declare function testFunction(s: string): P<string>;
const f: (x: number) => P<string> = testFunction;
const g: (x: number) => P<number> = testFunction;
declare function h(cb: (x: number) => P<string>): void;
h(testFunction);

interface Q<T> {
    then<U>(cb: (x: T) => Q<U>): Q<U>;
}
declare function qf(n: number): Q<number>;
declare function qf(s: string): Q<string>;
declare var q: Q<number>;
var next = q.then(qf);
