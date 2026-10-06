// @strict: false
class Base2<T> {
    x: T;
}
class Derived2<T> extends Base2<T> {
    constructor() {
        var r2 = () => super();
    }
}
class Derived3<T> extends Base2<T> {
    constructor() {
        var r = function () { super() }
    }
}
class Derived4<T> extends Base2<T> {
    constructor() {
        super();
    }
}
