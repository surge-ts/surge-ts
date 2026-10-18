// @strict: true
// A literal test of a fixed tuple's `length` narrows the reference `t.length`
// itself, while the tuple keeps its type; a reassigned tuple is not narrowed.
export function rest(...args: [pattern: string, value?: unknown]) {
  if (args.length === 1) {
    return args[0];
  }
  const two: 2 = args.length;
  if (args.length === 1) {
    return "unreachable";
  }
  return two;
}
export function differs(t: [string, number?]) {
  if (t.length !== 2) {
    const one: 1 = t.length;
    const two: 2 = t.length;
    return one + two;
  }
  return 0;
}
export function cases(t: readonly [string, number?]) {
  switch (t.length) {
    case 1: {
      const one: 1 = t.length;
      return one;
    }
    default: {
      const two: 2 = t.length;
      return two;
    }
  }
}
export function reassigned(u: [string, number?]) {
  if (u.length === 1) {
    return;
  }
  u = ["x"];
  const either: 1 | 2 = u.length;
  const two: 2 = u.length;
  return either + two;
}
