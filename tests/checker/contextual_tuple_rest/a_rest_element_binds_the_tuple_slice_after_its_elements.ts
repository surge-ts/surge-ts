// @strict: true
// tsc's `sliceTupleType`: a rest element binds the source tuple's elements
// from its own position on, a variadic tail gathered into an array, in a
// declaration's pattern and a parameter's alike.
type Encoded = [[unknown] | [], ...[string, number][]];
type Lead = [string, ...number[]];

export function declared(value: Encoded, lead: Lead) {
  const [[data], ...rest] = value;
  const r0: [string, number] = rest[0];
  const n: number = rest;
  const [s, ...nums] = lead;
  const x: string = nums;
  const [a, b, ...past] = lead;
  const p: string = past;
}

export function parameter([s, ...nums]: Lead, [flag, ...others]: [boolean, string, number]) {
  const x: string = nums;
  const o: number = others;
  const first: string = others[0];
}
