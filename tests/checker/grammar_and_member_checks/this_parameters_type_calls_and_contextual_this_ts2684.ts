// @noImplicitThis: true
class C {
    n = 101;
    explicitThis(this: C, m: number): number { return m + this.n; }
    explicitVoid(this: void, m: number): number { return m; }
}
interface I {
    n: number;
    explicitThis(this: I, m: number): number;
}
declare const c: C;
declare const i: I;

c.explicitVoid = c.explicitThis;

const o: I = {
    n: 1001,
    explicitThis: function (m) {
        return m + this.n.length;
    },
};

let x = i.explicitThis;
let n = x(12);
i.explicitThis(1);

function needsC(this: C) { return this.n; }
needsC();
c.explicitThis(2);
