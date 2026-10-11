// @declaration: true
// @noEmitOnError: true
// `noEmitOnError` skips emit when the program reports anything, and with it
// what declaration emit would have found: only TS2322 is reported.
export var noPrivates = class {
    private p = 12
}
const n: number = "x";
