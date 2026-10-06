// The edge where a `??`, `||` or `&&` skips its right operand passes through the
// left's condition, so a binding the right operand assigns joins the assigned
// type with the left's narrowing.
declare let source: string | undefined;

function coalesce(d: string | undefined) {
    d ?? (d = source ?? "x");
    d.length;
}
function or(d: string | undefined) {
    d || (d = "x");
    d.length;
}
function and(d: string | undefined) {
    d && (d = undefined);
    d.length;
}
function conditional(d: string | undefined, flag: boolean) {
    d || (flag && (d = "x"));
    d.length;
}

let topLevel: string | undefined;
topLevel ?? (topLevel = source ?? "x");
topLevel.length;

let other: string | undefined;
other ?? (other = source);
other.length;
