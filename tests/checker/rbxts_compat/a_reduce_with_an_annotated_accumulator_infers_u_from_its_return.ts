// `reduce(callbackfn, initialValue: T)` takes only an initial value the
// element type admits; otherwise `reduce<U>` infers `U` from the widened
// initial value and, when the accumulator parameter is annotated, from the
// callback's return too (`getCommonSupertype`).
export {};
const xs = ["a", "b"];
const r1 = xs.reduce((previous: undefined | number, current, index, arr) => index + (previous || index), undefined);
const c1: string = r1;
const r2 = xs.reduce((previous: number, current) => previous + current.length, 0);
const c2: string = r2;
const r3 = xs.reduce((previous, current) => previous + current);
const c3: number = r3;
const r4 = [1, 2].reduce((acc: number[], x) => [...acc, x], []);
const c4: string = r4;
const r6 = [[1], [2]].reduce((acc, x) => acc.concat(x), []);
const c6: string = r6;
const r7 = xs.reduce((n, s) => n + s.length, 0);
const c7: string = r7;
const r8 = xs.reduce((acc: Record<string, number>, s) => ({ ...acc, [s]: 1 }), {});
const c8: string = r8;
