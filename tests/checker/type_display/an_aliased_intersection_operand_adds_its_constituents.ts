// @surge-compare: messages
// @strict: true
type Tag1 = { __tag1__: void };
type Tag2 = { __tag2__: void };
type TaggedString1 = string & Tag1;
type TaggedString2 = string & Tag2;
declare let s0: string;
declare let both: TaggedString1 & TaggedString2;
declare let keyed: { [key: TaggedString1 & TaggedString2]: string };
keyed[s0];
let n1: number = both;
let s: string = both;
let len: number = both.length;
declare let disjoint: TaggedString1 & number;
let v: { x: 1 } = disjoint;
declare let repeated: string & Tag1 & string & Tag2;
let n2: number = repeated;
declare let arms: (TaggedString1 | number) & Tag2;
let n3: { y: 1 } = arms;
interface A { a: string }
interface B { b: string }
type AC = A & { z: 1 };
declare let ac1: AC & B;
let n4: number = ac1;
declare let ac2: A & AC;
let n5: number = ac2;
type Deep = AC & { d: 1 };
declare let deep: Deep & B;
let n6: number = deep;
type Both = TaggedString1 & TaggedString2;
declare let named: Both;
let n7: number = named;
interface X extends AC {}
declare let x1: X & B;
let n8: number = x1;
declare let props: { x: TaggedString1 } & { x: TaggedString1 };
let n9: number = props.x;
