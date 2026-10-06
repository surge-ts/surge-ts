// @strict: false
function fnReturn1() {
    return fnReturn1();
}
var a: any;
var a = fnReturn1();

function g(n: number) {
    if (n) return g(n - 1);
    return "s";
}
var d = g(1);
var d: number;

function shadowed(shadowed: () => number) {
    return shadowed();
}
var e = shadowed(() => 1);
var e: string;

async function h(n: number) {
    if (n) return await h(n - 1);
    return 1;
}
var p = h(1);
var p: Promise<string>;
