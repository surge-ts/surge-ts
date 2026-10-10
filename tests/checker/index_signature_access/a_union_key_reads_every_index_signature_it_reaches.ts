// `getIndexedAccessType` distributes a `string | number` key over the index
// signatures: the read is the union of both, a write their intersection.
class A {
    a!: number;
}
class B extends A {
    b!: number;
}
declare var both: { [s: string]: A; [n: number]: B };
declare var key: string | number;
var read = both[key];
var read: A;
