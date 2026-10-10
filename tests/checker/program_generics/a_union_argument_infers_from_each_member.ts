// @target: es2022
// A union argument infers into a structural parameter from each of its
// members (`inferFromTypes`); the candidates then settle on their common
// supertype, which the other members fail.
declare function h<V>(a: { x: V }[]): V;
const o = h([{ x: true }, { x: 0 }]);
const p: string = o;
declare function k<V>(a: readonly [string, V][]): V;
const q: number = k([["", 1], ["", 2]]);
