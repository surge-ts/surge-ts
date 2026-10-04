function foo(x: number | string) {
    function f() {
        x = 10;
    }
    if (typeof x === "string") {
        f();
        return x.length;
    }
    else {
        return x++;
    }
}
function foo2(x: number | string) {
    if (typeof x === "string") {
        return x.length;
    }
    else {
        var f = function () {
            return x * x;
        };
    }
    x = "hello";
    f();
}
function foo3(x: number | string) {
    if (typeof x === "string") {
        return x.length;
    }
    else {
        var f = () => x * x;
    }
    x = "hello";
    f();
}
function keeps(x: number | string) {
    x = 1;
    if (typeof x === "number") {
        const g = () => x * x;
    }
}
function nested(x: number | string) {
    if (typeof x === "number") {
        const g = () => { x = "s"; };
        const h = () => x * x;
    }
}
