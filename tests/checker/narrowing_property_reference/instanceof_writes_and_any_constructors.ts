interface A { foo: string }
declare var A: { new (): A };
declare var o: any;
if (o instanceof A) {
    o.bar = "str";
    o.foo = 1;
}

interface FConstructor { new (): any; }
interface F { foo: string; bar: number; }
declare var F: FConstructor;
declare var obj11: F | string;
if (obj11 instanceof F) {
    obj11.foo;
}
function g(p: F | string) {
    if (p instanceof F) {
        p.bar;
    }
}
