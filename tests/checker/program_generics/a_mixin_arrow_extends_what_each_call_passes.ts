// @strict: false
type Constructor<T> = new (...args: any[]) => T;
class Base { constructor(public x: number, public y: number) {} }
class Derived extends Base { constructor(x: number, y: number, public z: number) { super(x, y); } }
const Printable = <T extends Constructor<Base>>(superClass: T) => class extends superClass {
    print() { return this.x + "," + this.y; }
};
function Tagged<T extends Constructor<{}>>(superClass: T) {
    class C extends superClass {
        _tag: string;
    }
    return C;
}
const P = Printable(Derived);
const p = new P(1, 2, 3);
const z: number = p.z;
const printed: string = p.print();
const Thing = Tagged(Printable(Derived));
const thing = new Thing(1, 2, 3);
thing.print();
thing._tag;
thing.missing;
class Thing3 extends Thing {
    test() {
        this.print();
        this.nope();
    }
}
