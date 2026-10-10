// @strict: true
// A method called on a type variable is its constraint's member, unless it
// reads the polymorphic `this` the variable would instantiate. An array
// literal's element union drops a type variable its constraint subsumes.
class Store<T> {
  plain(): T { return null!; }
  str(): string { return ""; }
  self(): this { return this; }
  field!: T;
}
class Plain {
  str(): string { return ""; }
}
function f<S extends Store<string>, P extends Plain>(s: S, p: P) {
  const a: number = s.str();
  const b: number = s.field;
  const m = s.str;
  const c: number = m();
  const d: number = p.str();
  const e: number = s.plain();
  const g: S = s.self();
  return [a, b, c, d, e, g];
}
function h<T extends Plain>(t: T, plain: Plain) {
  var both: Plain[];
  var both = [t, plain];
  return both;
}
