// @noImplicitAny: true
function*foo(a = yield) {
}
function*bar() {
    function*baz(b = yield, c: number = yield, d: any = yield) {
    }
}
function*typed(e = yield): Generator<undefined, void, string> {
}
