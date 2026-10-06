declare function f(cb: ((x: string) => void) & ((x: number) => void)): void;
f(x => { const n: boolean = x; });
declare function g(cb: ((x: string) => void) & ((x: never) => void)): void;
g(x => { const n: boolean = x; });

declare const input: string;
const words = input.match(/[\da-z]+/g) ?? [];
words.map((word) => word.toLowerCase()).join('_');
words.reduce((acc, word) => acc + word.slice(1), '');
