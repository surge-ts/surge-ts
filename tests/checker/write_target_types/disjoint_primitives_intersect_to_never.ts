// @strict: true
// @target: es2020
// Primitives of disjoint domains, or two distinct unit types, intersect to
// `never` (`getIntersectionType`); a write through a union key targets the
// intersection of the members' write types.
declare const a: string & null;
const t1: "x" = a;
declare const b: (boolean | string) & (boolean | null);
const t2: "x" = b;
declare const c: "a" & "b";
const t3: "x" = c;
declare const d: string & "a";
const t4: "x" = d;
declare const e: void & undefined;
const t5: "x" = e;
class Six {
  get prop1(): boolean | number { return 42; }
  set prop1(s: boolean | string) {}
  get prop2(): bigint | number { return 10; }
  set prop2(s: boolean | null) {}
}
declare const s1: Six;
declare const k1: 'prop1' | 'prop2';
s1[k1] = true;
s1[k1] = '';
