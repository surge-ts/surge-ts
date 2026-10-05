// @target: esnext
// @lib: esnext
export {};
async function f() {
    await using a = {};
    await using b = null;
    await using c = { [Symbol.asyncDispose]: async () => {} };
    using d = {};
    using e = { [Symbol.dispose]() {} };
}
