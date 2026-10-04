// @target: esnext
declare const g1: Generator<number, void, string>;
declare const g2: Generator<number, void, undefined>;

let [x1] = g1;
let [x2] = g2;
let [...y1] = g1;
let [...y2] = g2;
let [a1, b1] = g1;
