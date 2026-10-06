// @strict: true
declare function obj(x: object): void;
function f6(x: unknown) {
    if (x === null) {
        x;
    }
    else {
        x;
    }
    if (typeof x === 'object') {
        obj(x);
    }
    if (typeof x === 'object' && x !== null) {
        obj(x);
    }
}
