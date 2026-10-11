// @strict: true
// `fn.x = …` declares an expando member only where `fn` is a local of the
// container the write sits in (`lookupSymbolForPropertyAccess`): at the top
// level, its blocks included. A write from a nested function, an arrow or a
// parameter default declares nothing, and a member the function lacks is
// TS2339 there.
function Foo(): void {}
Foo.ok = 1;
if (Foo.ok) {
    Foo.inIf = 1;
}
function g() { Foo.inBody = 1; }
function bar(p = (Foo.inParam = 1)) {}
const arrow = () => { Foo.inArrow = 1; };

function outer() {
    function inner() {}
    inner.x = 1;
    const nested = () => { inner.y = 1; };
    return inner.x;
}
