// @strict: true
// @noUncheckedIndexedAccess: true
// @lib: es2023
type A = { type: "a"; x: number };
type B = { type: "b"; y: string };
declare const xs: (A | B)[];
declare const node: { arguments: (A | B)[] };
declare const sink: { push(item: { value: A }): void };
if (xs[1]?.type === "a") {
  const viaChain: A = xs[1];
}
if (node.arguments.length > 1 && node.arguments[1]?.type === "a") {
  sink.push({ value: node.arguments[1] });
}
if (xs[0]?.type === "b") {
  const wrong: A = xs[0];
}
