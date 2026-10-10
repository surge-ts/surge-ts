// @strict: false
// @noImplicitAny: true
// A generator with no return annotation and no contextual signature gives
// its `yield` and `yield*` operands no contextual type: a nameless generator
// called there still reports its implicit `any` yield type (TS7025), and an
// arrow there its untyped parameter (TS7006).
function* g311() {
    yield* (function*() {
        const y: string = yield;
    })();
}
function* g1() {
    yield* [(x) => x];
}
declare function take(g: () => Generator<(x: number) => number>): void;
take(function* () { yield (x) => x; });
