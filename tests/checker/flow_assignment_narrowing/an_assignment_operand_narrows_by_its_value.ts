// An assignment tested as a logical operand narrows by its value first, then
// by its target (`narrowTypeByBinaryExpression`).
declare function onlyFalse(value: false): void;
function truthy(x: false) {
    let y: boolean;
    return (y = x) && x.toString();
}
function falsy(x: boolean) {
    let z: boolean;
    return (z = x) || onlyFalse(x);
}
function mixed(x: number | string | boolean) {
    let y: number | boolean | string;
    let z: number | boolean | string;
    return typeof x === "string"
        || ((z = x)
        || (typeof x === "number"
            ? ((x = 10) && x.toString())
            : ((y = x) && x.toString())));
}
