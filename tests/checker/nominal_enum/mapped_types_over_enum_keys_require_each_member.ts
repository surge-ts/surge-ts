enum E { A }
let foo: Record<E, any> = {};
enum F { A, B = 5 }
let bar: Record<F, number> = { [F.A]: 1 };
let baz: { [K in E]: any } = {};
enum S { X = "x", Y = "y" }
let ok: Record<S, number> = { x: 1, y: 2 };
let missing: { [K in S]: K } = { x: S.X };
