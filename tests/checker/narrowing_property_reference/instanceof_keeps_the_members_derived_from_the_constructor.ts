// @surge-compare: messages
class A<T> {
    constructor(private a: string) { }
}
class B<T> { }
function acceptA(a: A<number>) { }
function anEmptyClassIsNotABaseOfEveryClass<T>(x: A<T> | B<T>) {
    if (x instanceof B) {
        acceptA(x);
    }
}

class Base { }
class Mid extends Base { mid = 1 }
class Leaf extends Mid { leaf = 1 }
class Other { other = 1 }
function derivationFollowsTheExtendsChain(x: Leaf | Other) {
    if (x instanceof Base) {
        const n: string = x;
    } else {
        const n: string = x;
    }
}
function aPropertyPathNarrowsTheSameWay(o: { value: Leaf | Other }) {
    if (o.value instanceof Mid) {
        const n: string = o.value;
    } else {
        const n: string = o.value;
    }
}

interface Shape { size: number }
class Square implements Shape { size = 1; side = 1 }
class Circle { size = 1 }
function implementingIsNotDeriving(x: Square | Circle) {
    if (x instanceof Circle) {
        const n: string = x;
    } else {
        const n: string = x;
    }
}

class P { a = 1 }
class Q { a = 1; b = 2 }
function aStructuralSupersetStaysInTheFalseBranch(x: Q | number) {
    if (!(x instanceof P)) {
        return x === "s";
    }
}

class ParseError extends Error { line = 1 }
function aLibraryBaseIsReachedToo(e: ParseError | { message: string; code: number }) {
    if (e instanceof Error) {
        const n: string = e;
    } else {
        const n: string = e;
    }
}

function everyObjectDerivesFromObject(thing: string | number | Date) {
    if (!(thing instanceof Object)) {
        return thing === true;
    }
}
