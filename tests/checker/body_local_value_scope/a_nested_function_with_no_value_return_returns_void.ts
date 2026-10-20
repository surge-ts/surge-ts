// @strict: true
// A function declared in a body whose `return`s carry no value returns
// `void`, as a top-level one does; reading it was `any`.
export function outer() {
    function noReturn() {}
    function bareReturn(flag: boolean) {
        if (flag) return;
    }
    async function asyncNoReturn() {}
    const a: number = noReturn();
    const b: number = bareReturn(true);
    const c: number = asyncNoReturn();
    noReturn.extra = 1;
    const d: number = noReturn();
    return [a, b, c, d];
}
