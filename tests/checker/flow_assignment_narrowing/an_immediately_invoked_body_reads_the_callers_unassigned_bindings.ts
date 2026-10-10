// @strict: true
// An immediately invoked body is part of its caller's flow: a binding still
// unassigned at the call is unassigned in it, which it is not in a closure
// called later.
function f(param: string | number) {
    var a: string | number;
    var r = function (p: string | number) {
        return typeof a === "string" && a.length;
    }(param);
    var b: string;
    (() => b.length)();
    var c: string;
    c = "";
    (() => c.length)();
    var d: string;
    const later = () => d.length;
    (async () => { d.length; })();
    var e: string;
    (function (e: string) { return e.length; })("");
}
export {};
