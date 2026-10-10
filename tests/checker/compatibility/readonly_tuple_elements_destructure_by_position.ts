// @strict: true
// @lib: es2023
declare const pairs: (readonly [string, number])[];
for (const [name, size] of pairs) {
  const wrongName: number = name;
  const wrongSize: string = size;
}
declare const iterable: Iterable<readonly [string, number]>;
for (const [key] of iterable) {
  const wrongKey: number = key;
}
const entry = ["a", 1] as const;
const [first, second] = entry;
const wrongFirst: number = first;
const wrongSecond: string = second;
function take([a, b]: readonly [string, boolean]) {
  const wrongA: number = a;
  const wrongB: string = b;
}
declare const open: readonly [string, ...number[]];
const [head, ...tail] = open;
const wrongHead: number = head;
