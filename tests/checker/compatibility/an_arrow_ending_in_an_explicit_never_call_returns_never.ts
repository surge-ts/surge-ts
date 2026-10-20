// @strict: true
// An arrow whose end a `never` call makes unreachable returns `never`, as
// tsc's `mayReturnNever` reads it. The callee counts only through a name
// with an explicit type (`getExplicitTypeOfSymbol`); an unannotated `const`
// gives the call no effects, so that arrow returns `void`.
function fail(): never {
    throw new Error();
}
class Stopper {
    stop(): never {
        throw new Error();
    }
}
const inferredFail = (): never => {
    throw new Error();
};
export const direct: () => string = () => {
    fail();
};
export function viaParameter(stopper: Stopper) {
    const viaMethod: () => string = () => {
        stopper.stop();
    };
    const local = stopper;
    const viaLocal: () => string = () => {
        local.stop();
    };
    return [viaMethod, viaLocal];
}
export const viaInferred: () => string = () => {
    inferredFail();
};
