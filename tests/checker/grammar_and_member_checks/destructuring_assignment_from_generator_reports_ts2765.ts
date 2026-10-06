// @target: esnext
declare let _: any;
declare const g1: Generator<number, void, string>;
declare const g2: Generator<number, void, undefined>;

let [x1] = g1;
let [x2] = g2;

[_] = g1;
[_] = g2;

[..._] = g1;
[..._] = g2;

[, _] = g1;
