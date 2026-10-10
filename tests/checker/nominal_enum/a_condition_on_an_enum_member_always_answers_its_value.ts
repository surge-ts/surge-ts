// @strict: true
// An enum member read through its enum has a known value, so a condition
// on it always returns that value's truthiness (`checkTestingKnownTruthyType`).
const enum E { A, B }
enum F { A = "", B = "b" }
namespace N {
    export const enum G { A, B }
}
E.A ? 1 : 2;
E.B ? 1 : 2;
if (F.A) {}
if (F.B) {}
N.G.A ? 1 : 2;
declare const e: E;
if (e) {}
namespace Outer {
    import Inner = N;
    Inner.G.B ? 1 : 2;
    N.G.A ? 1 : 2;
    export class Holder {
        static readonly A = 0;
        constructor() {
            Inner.G.A ? "a" : "b";
            Holder.A ? 1 : 2;
        }
    }
}
