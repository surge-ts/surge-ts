declare const t: [number];
const l: 7 = t.length;
type B = Pick<[number], 'length'>;
declare const b: B;
const m: 7 = b.length;
b.length = 0;
declare const s: unique symbol;
declare const symbolMap: { [s]: string };
symbolMap[s] = undefined;
