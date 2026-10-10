// @target: esnext
// @lib: esnext
// `IteratorObject` (and what derives from it) is `Disposable`, a plain
// `Iterator` is not: `using` one is TS2741 against `Disposable`, and
// `await using` one TS2851.
declare const i: Iterator<string, undefined>;
declare const io: IteratorObject<string, undefined, unknown>;
declare const g: Generator<string, void>;

function f() {
    using it0 = io;
    using it1 = g;
    using it2 = [].values();
    using it7 = i;
}

async function h() {
    await using it0 = io;
    await using it7 = i;
}
