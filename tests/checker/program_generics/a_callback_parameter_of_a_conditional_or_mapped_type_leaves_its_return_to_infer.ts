// @strict: true
// A callback parameter typed by a conditional, mapped or template literal
// type mentions only what its parts do, so a type parameter only the
// callback's return names is still inferred from it.
declare function mc<T, R>(obj: T, cb: (value: T extends string ? T : never) => R): R;
mc("a", v => v.length).toFixed();
declare function mm<T, R>(obj: T, cb: (value: { [K in keyof T]: T[K] }) => R): R;
mm({ a: 1 }, v => v.a).toFixed();
declare function mt<T extends string, R>(obj: T, cb: (value: `pre-${T}`) => R): R;
mt("a", v => v.length).toFixed();
