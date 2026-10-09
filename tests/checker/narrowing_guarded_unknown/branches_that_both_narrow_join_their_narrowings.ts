// @strict: true
declare function obj(x: object): void;
declare const c: boolean;
function both(x: unknown) {
    if (c) { if (!x) { return; } } else { if (x === null) { return; } }
    if (typeof x === "object") { obj(x); }
    const n: number = x;
}
function one(x: unknown) {
    if (c) { if (!x) { return; } }
    const n: number = x;
}
function constant(x: unknown) {
    if (true) { if (!x) { return; } } else { if (x === null) { return; } }
    if (typeof x === "object") { obj(x); }
}
function strings(s: string | undefined) {
    if (c) { if (!s) return; } else { if (!s) throw new Error(); }
    s.length;
}
function different(s: string | number | boolean) {
    if (c) { if (typeof s === "boolean") return; } else { if (typeof s === "number") return; }
    const n: number = s;
}
