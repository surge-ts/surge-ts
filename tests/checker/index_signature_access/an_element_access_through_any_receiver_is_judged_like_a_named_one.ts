// @strict: true
const lit = "nope";
declare const k: string;
const o = { p: { a: 1 } };
o.p[k];
o.p[lit];
o.p[k] = 1;
o.p[lit] = 1;
const q = { a: 1 };
q[k] = 1;
function f() { return { a: 1 }; }
f()[k];
f()[lit];
f()[k] = 1;
const r: Record<string, number> = {};
r[k] = 1;
const s = { m: r };
s.m[k] = 2;
s.m[k];
const t = [1, 2];
const u = { list: t };
u.list[0];
u.list[k as any];
