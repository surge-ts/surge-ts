namespace M3 {
    export enum Color { Blue, Red }
}
var m3: typeof M3;
var p3: M3.Color;
var p3 = M3.Color.Red;
var p3 = m3.Color.Blue;
var a3: typeof M3.Color;
var b = a3.Blue;
var b: M3.Color;
const n: string = m3.Color.Red;
