declare const o: { a: 1 | 3; c: "x"; d: true };
o.a = 2;
o.c = "y";
o.d = false;
o.a = 3;

declare const t: [number];
t.length = 0;

function f(p: { a: 1 }, w: { k: string }) {
  p.a = 2;
  if (w.k === "a") {
    w.k = "b";
  }
}

export { f };
