declare function fn5(f: (n: string) => void): string;
declare function fn5(f: (n: number) => void): number;
var n = fn5((n) => n.toFixed());
var s = fn5((n) => { const x: boolean = n; });

declare function fn6(a: 1, f: (n: string) => void): string;
declare function fn6(a: 2, f: (n: number) => void): number;
var m = fn6(2, (n) => { const x: boolean = n; });

declare function g(cb: (x: string) => number): 1;
declare function g(cb: (x: number) => string): 2;
g(x => x.toFixed());
