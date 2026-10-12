// @strict: false
declare var a: {
    (x: boolean): boolean;
    (x: string): string;
};
declare function foo4(cb: typeof a): void;
foo4(<T, U>(x: T) => { var r: U; return r });
declare function foo6<T>(cb: { (x: T): string; (x: T, y?: T): string }): void;
foo6(<T>(x: T, y: T) => '');
declare function g(cb: { (x: number): string; (x: number, y?: number): string }): void;
g((x: number, y: number) => '');
g((x: number, y?: number) => '');
