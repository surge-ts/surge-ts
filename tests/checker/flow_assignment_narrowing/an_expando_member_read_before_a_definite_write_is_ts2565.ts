// @strict: true
// An expando member is unassigned until a write to it definitely ran
// (`getFlowTypeOfAccessExpression`), so a read that only some paths reach
// written is TS2565. A read in another function is not checked.
function f(b: boolean) {
    function d() {}
    d.e = 12;
    d.e;
    if (b) {
        d.q = false;
    }
    d.q;
    if (b) {
        d.r = 1;
    } else {
        d.r = 2;
    }
    d.r;
    return d;
}
function d() {}
d.e = 12;
d.e;
if (!!false) {
    d.q = false;
}
d.q;
const g = function () {};
if (!!false) {
    g.expando = 1;
}
g.expando;
function later() {
    return g.expando;
}
