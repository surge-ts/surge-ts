// @strict: true
// `typeof this.x` in a class property's annotation or a member's body reads
// the member off the declaring class, as `this['x']` does there.
class Accepted {
  x = 1;
  o = { deep: "s" };
  b: typeof this.x = 0;
  c: typeof this.o.deep = "";
  m() { const s: string = this.b; const n: number = this.c; let local: typeof this.x = 2; const t: string = local; }
}
export {};
