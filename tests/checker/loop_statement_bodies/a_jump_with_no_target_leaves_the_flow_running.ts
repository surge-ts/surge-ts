// @strict: true
// @allowUnreachableCode: false
// A `break` or `continue` with no target is a grammar error and leaves the
// flow running (`bindBreakOrContinueFlow` finds no label to end it at).
export {};
const ready = true;
if (ready) {
    break;
}
const afterBreak: string = ready;
for (;;) {
    const f = () => {
        break;
        return 1;
    };
    const n: string = f();
    block: {
        continue block;
    }
    const m: string = 1;
    break;
}
