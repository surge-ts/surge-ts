declare const a: string[] & { extra: number };
const b: string[] = a;
declare function f(x: string[]): void;
f(a);
a.extra; a.length; a[0].toUpperCase();
