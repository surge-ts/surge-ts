// @declaration: true
// `Program.Emit` runs under `noEmit` too, so what declaration emit finds is
// reported beside the program's other errors (TS4094 with TS2322).
export var noPrivates = class {
    private p = 12
}
const n: number = "x";
