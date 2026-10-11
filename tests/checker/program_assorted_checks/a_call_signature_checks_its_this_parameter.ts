type A = { a: string };
type B = { b: number };
type C = { c: string };
type D = { d: number };

interface Single { (this: A): void }
declare var single: Single;
single();

type Literal = { (this: A): number };
declare var literal: Literal;
literal();

interface Overloaded { (this: A): void; (this: B): void }
declare var overloaded: Overloaded;
overloaded();

interface ByArity { (this: A): void; (x: number): void }
declare var byArity: ByArity;
byArity();

interface F3 { (this: A): void; (this: B): void }
interface F4 { (this: C): void; (this: D): void }
interface F5 { (this: C): void; (this: B): void }
declare var x1: A & C & { f3: F3 | F4; f4: F3 | F5 };
x1.f3();
x1.f4();

declare var x2: A & B & { f4: F3 | F5 };
x2.f4();
