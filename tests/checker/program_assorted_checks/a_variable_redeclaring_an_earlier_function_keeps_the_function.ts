declare function foo(x: number): void;
const foo = { bar: 'a' };
foo.bar;
foo(1);

class Bar { m() { } }
const Bar = 1;
new Bar().m();

const baz = 1;
function baz() { }
baz.toFixed();
