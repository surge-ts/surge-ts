// @strict: true
// An object type literal is no member, so a `this` inside it is TS2526 and
// reads as the error type, in a property's annotation, a method's return, a
// member body or an interface.
export class A {
  nested: { of: this } = null!;
  meth: { m(): this } = null!;
  literal(): { self: this } { return null!; }
  m() {
    const local: { a: this } = null!;
    const n1: number = this.nested.of;
    const n2: number = this.meth.m();
    const n3: number = this.literal().self;
    const n4: number = local.a;
  }
}
interface I { nested: { of: this } }
declare const i: I;
export const n5: number = i.nested.of;
