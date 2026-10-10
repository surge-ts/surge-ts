// An arithmetic operand of the wrong type is reported, and the operation is
// still `number` (`checkBinaryLikeExpressionWorker`).
function f(x: boolean) {
    return x * 2;
}
f(true).length;
const n: string = f(false);
