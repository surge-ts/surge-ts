// @surge-compare: messages
// A namespace's unannotated export is typed by its initializer, read through
// the namespace as a local `const` is (roblox-ts function.spec's `N.a`, whose
// `this: typeof N` names the namespace being typed). The initializer reads
// the block's own locals, over a same-named outer value, and what another
// block of the namespace exports.
// @filename: ns.ts
const x = "outer";
export namespace N {
    const x = 1;
    export const o = { p: x };
    export let l = (n: number) => n;
    export const a = function (this: typeof N, n: number) {
        return n;
    };
}
export namespace N {
    export const merged = { m: o };
}
const o: string = N.o;
const p: number = N.o.p;
// @filename: main.ts
import { N } from "./ns";
const l: string = N.l;
const a: string = N.a;
const m: string = N.merged;
N.a(1);
