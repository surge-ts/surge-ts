// @allowUnreachableCode: false
// @preserveConstEnums: true
// @noEmit: true
// @filename: a.ts
function f4() {
    if (true) {
        throw new Error();
    }
    const enum E {
        X = 1
    }
}
// @filename: b.ts
throw new Error();
namespace M {
    const enum E { A }
}
interface I {}
