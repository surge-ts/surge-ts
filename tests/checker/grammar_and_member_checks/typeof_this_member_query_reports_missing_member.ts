// @strict: true
class A {
  x = 1;
  y: typeof this.x = 2;
  z: typeof this.missing;
  m() { const q: typeof this.x = 3; return q; }
}
class B extends A {
  v: typeof this.x = 5;
  u: typeof this.nope;
  o: typeof this.x | typeof this.gone = 1;
}
class G<T> { t!: T; g: typeof this.t = null!; h: typeof this.k; }
