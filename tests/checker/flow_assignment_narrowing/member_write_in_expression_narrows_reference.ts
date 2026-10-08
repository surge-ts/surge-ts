type R = { promise: number };
declare function make(): R;
export class Q {
  #r?: R;
  p?: R;
  readonly ro: number;
  constructor() { const x = (this.ro = 1); }
  a() { const r = (this.#r = make()); return this.#r.promise + r.promise; }
  b() { (this.#r = make()); return this.#r.promise; }
  c() { const r = (this.p = make()); return this.p.promise; }
  d() { const o: { p?: R } = {}; (o.p = make()); return o.p.promise; }
  e(flag: boolean) { flag && (this.p = make()); return this.p.promise; }
  bad() { const r = (this.p = 1); }
  bad2() { const r = (this.ro = 2); }
}
