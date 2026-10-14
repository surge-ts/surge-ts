// @strict: true
// A fixed tuple's `length` is a literal discriminant.
export function insert<T>(list: T[], ...rest: [value: T] | [pos: number, value: T]): void {
  if (rest.length === 1) {
    list.push(rest[0]);
    return;
  }
  const [pos, value] = rest;
  const at: number = pos;
  list.splice(at, 0, value);
}
export function pair(rest: [string] | [number, string]) {
  if (rest.length === 2) {
    const n: number = rest[0];
  } else {
    const s: string = rest[0];
    const wrong: number = rest[0];
  }
}
