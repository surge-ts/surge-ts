// @declaration: true
// A `var` only a block declares is not written to the declaration file, so
// an exported variable or type alias naming it in `typeof` uses a private
// name (TS4025, TS4081); a top-level variable is written and stays nameable.
{
    var a = "";
}
if (Math.random()) {
    var c = 1;
}
var d = true;
export let b: typeof a;
export type T = typeof c | undefined;
export let e: typeof d;
