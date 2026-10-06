var a: string, b: boolean, c: boolean[];
[a, b] = { 0: "", 1: true };
[a, ...c] = { 0: "", 1: true };
declare const o: { 0: string; 1: boolean };
[a, b] = o;
declare const n: number;
[a] = n;
declare const m: string[] | undefined;
[a] = m;
for ([a, b] of [o]) {}
[[a]] = [o];
[, a] = o;
declare const ok: [string, boolean];
[a, b] = ok;
