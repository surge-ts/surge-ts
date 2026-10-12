// @strict: false
// @noImplicitAny: true
function nullReturn() { return null; }
function arrayReturn() { return [undefined]; }
function objectReturn() { return { a: null }; }
function mixed(b: boolean) { if (b) return null; return 1; }
function bare(b: boolean) { if (b) return null; return; }
function annotated(): any { return null; }
const arrow = () => null;
const anonymous = function () { return [null]; };
const named = function inner() { return null; };
class K {
    method() { return null; }
    get getter() { return null; }
    static staticMethod() { return undefined; }
    empty = [];
    nullish = null;
    object = { x: null };
    property = () => null;
}
const literal = { method() { return null; }, arrow: () => null, plain: function () { return null; } };
declare function take(f: () => number): void;
take(() => null);
function tuple() {
    let intermediate: [string];
    return intermediate = [undefined];
}
