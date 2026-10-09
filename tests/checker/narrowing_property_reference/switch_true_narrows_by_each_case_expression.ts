// @strict: true
type Shape = { kind: "circle"; r: number } | { kind: "square"; s: number } | { kind: "tri"; a: number; b: number };
function area(s: Shape) {
    switch (true) {
        case s.kind === "circle": return s.r;
        case s.kind === "square": return s.s;
        default: return s.a;
    }
}
function tags(x: string | number | boolean) {
    switch (true) {
        case typeof x === "string": { const s: string = x; break; }
        case typeof x === "number": { const n: number = x; break; }
        default: { const b: boolean = x; }
    }
}
function grouped(x: string | number | boolean) {
    switch (true) {
        case typeof x === "string":
        case typeof x === "number": { const sn: string | number = x; const s: string = x; break; }
        default: { const b: boolean = x; }
    }
}
function preceding(x: string | number | undefined) {
    switch (true) {
        case x === undefined: return;
        case typeof x === "string": { const s: string = x; break; }
        default: { const n: number = x; }
    }
}
