// @isolatedModules: true
// @filename: a.ts
const d = 'd';
const e = 'e';
enum F { G = e }
// @filename: b.ts
enum E {
    D = d,
    P = d + "x",
    Q = "x" + d,
}
enum H { I = e }
// @filename: c.ts
export {};
enum K { L = d }
