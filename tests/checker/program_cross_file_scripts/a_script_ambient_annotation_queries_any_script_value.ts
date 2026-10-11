// @strict: true
// A script's `declare` variable is typed on demand, so a type query in its
// annotation reads any value the scripts declare — a function, a namespace, an
// ambient or initialized variable — not only one published before it.
function f() { return 1; }
namespace N { export const v = 1; }
declare const x: number;
const y = 2;
declare const g1: typeof f;
declare const g4: typeof N;
declare const g5: typeof x;
declare var g6: typeof y;
declare let g7: typeof N.v;
const g8: string = g1;
const g9: string = g5;
const g10: string = g7;
