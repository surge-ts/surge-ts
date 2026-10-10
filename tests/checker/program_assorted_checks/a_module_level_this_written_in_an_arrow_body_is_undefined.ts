// @strict: true
// A module's top level owns `this`, which is `undefined`, and an arrow sees
// through to it: writing a member of it is TS2532 like reading one.
export {};
const f1 = () => { return this.a; };
const f2 = (input: number) => { this.b = input; };
const f3 = () => { this.c.d = 1; };
