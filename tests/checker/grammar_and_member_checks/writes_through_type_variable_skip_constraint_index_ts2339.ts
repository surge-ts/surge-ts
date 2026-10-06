function f2<T extends { [key: string]: number }>(c: T) {
    c.x;
    c.x = 1;
}

type Dict = Record<string, number>;

function f12<T extends Readonly<Dict>>(obj: T) {
    obj.foo = 123;
}

function f13<T extends Dict & { known: number }>(obj: T) {
    obj.known = 1;
    obj.toString = () => "";
    if (obj.known) {
        obj.bar = 2;
    }
}

class K<T extends Dict> {
    constructor(public t: T) {
        this.t.q = 1;
    }
}

function f14(d: Dict) {
    d.free = 1;
}
