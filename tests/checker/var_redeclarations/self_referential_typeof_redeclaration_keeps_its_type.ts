// @strict: true
var h2: new () => typeof h2;
var h2 = new h2();
var i2: new (x: typeof i2) => typeof x;
var i2 = new i2(i2);
var hy1: { x: typeof hy1 }[];
var hy1 = hy1[0].x;
var hy2: { x: Array<typeof hy2> };
var hy2 = hy2.x[0];
var other: { x: number };
var other: { x: string };
namespace B {
    export function Point() {
        return { x: 0, y: 0 };
    }
    export namespace Point {
        export var Origin = { x: 0, y: 0 };
    }
}
var fn: () => { x: number; y: number };
var fn = B.Point;
