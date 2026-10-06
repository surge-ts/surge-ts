// @allowUnreachableCode: false
function f1() {
    throw 1;
    const g = function () { return 1; let y = 2; };
    if (g) { let z = 1; }
}
function f2(x: number) {
    if (x) {
        (() => { throw 1; })();
        x++;
    }
    return x;
}
function f3(x: number) {
    x = (function () { throw 1; })() + (function () { return 1; })();
}
function f4(x: number) {
    x = x && (function () { throw 1; })();
    x++;
}
function f5() {
    for (;;) {}
    [1].map(function () { return 1; });
    (function () { return 2; })();
    (async function () { return 2; })();
    (() => 3)();
}
function f6() {
    (function () { throw "1"; })();
    (function () { return 1; })();
}
function f7(x: number) {
    for (; x; (function () { return 1; })()) { throw 1; }
}
function f8(x: number) {
    while ((function () { throw 1; })()) { x++; }
    x--;
}
function f9() {
    try {
        for ((function () { throw "1"; })(); (function () { throw "2"; })(); (function () { throw "2"; })()) { f9(); }
    } catch {}
}
function f10() {
    throw 1;
    class C { m() { return; let q = 1; } }
    const D = class { m() { return; let q = 1; } };
    const o = { m() { return; let q = 1; }, get g() { return 1; let r = 2; } };
}
function f11(x: number) {
    if ((() => { throw 1; })()) { x++; } else { x--; }
    x++;
}
