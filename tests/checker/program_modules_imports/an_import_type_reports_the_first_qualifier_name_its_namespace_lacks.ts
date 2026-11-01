// @module: commonjs
// @filename: foo2.ts
namespace Bar { export interface I { a: string; } }
class Bar {}
export { Bar };
export enum E { A, B }
export namespace V { export const x = 1; }
export namespace Direct { export namespace Inner { export interface I { a: string; } } }
namespace Hidden { export interface I { a: string; } }

// @filename: usage.ts
export let a1: import("./foo2").Bar.I = { a: "" };
export let a2: import("./foo2").Bar.Q;
export let e1: import("./foo2").E.Q;
export let v1: import("./foo2").V.Q;
export let v2: import("./foo2").V.x.Q;
export let d1: import("./foo2").Direct.Q;
export let d2: import("./foo2").Direct.Inner.Q;
export let d3: import("./foo2").Direct.Q.R;
export let h1: import("./foo2").Hidden.I;
