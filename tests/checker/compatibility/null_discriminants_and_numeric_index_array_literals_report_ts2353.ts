var context1: { [n: number]: { a: string; b: number; }; } = [{ a: '', b: 0, c: '' }, { a: "", b: 3, c: 0 }];
var context2: { [n: number]: { a: string; b: number; }; } = [{ a: '', b: 0 }];

function f13(x: { a: null; b: string } | { a: string, c: number }) {
    x = { a: null, b: "foo", c: 4 };
    x = { a: null, b: "foo" };
    x = { a: "s", c: 1 };
}
