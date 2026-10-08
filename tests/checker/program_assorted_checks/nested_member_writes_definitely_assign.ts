// @strictPropertyInitialization: true
const A = Symbol('a');
const B = Symbol('b');

export class Table {
  [A]: string;
  [B]: string;
  plain: string;
  other: string;
  missing: string;

  constructor(name: string) {
    this[A] = this[B] = name;
    this.plain = this.other = name;
  }
}
