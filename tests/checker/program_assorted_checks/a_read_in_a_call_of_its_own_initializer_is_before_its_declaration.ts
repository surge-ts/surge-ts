// @strict: true
// @target: es2015
function t1() {
    const x = (() => {
        try {
            return 'x';
        }
        catch (e) {
            return null;
        }
        x;
    })();
    x;
}
function t3() {
    const z = (() => {
        return 1;
        z;
    })();
}
function t4() {
    const w = (() => {
        const w = 2;
        return w;
    })();
    const v = (function () { return () => v; })();
    var s = (() => { return 1; s; })();
}
