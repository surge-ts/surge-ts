// @noImplicitAny: true
const _sym = Symbol("_sym");
export class MyClass {
    declare known: number;
    constructor() {
        const self = this;
        self[_sym] = "ok";
    }
    method() {
        const self = this;
        self[_sym] = "yep";
        this[_sym] = 1;
        const x = self[_sym];
    }
}
const instance = new MyClass();
instance[_sym] = 2;
