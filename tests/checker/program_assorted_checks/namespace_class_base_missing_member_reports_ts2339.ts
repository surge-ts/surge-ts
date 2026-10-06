namespace M {
    export class E<T> { foo!: T }
}
class D2 extends M.C { }
class D3 extends M.E<number> { }
declare namespace N {
    export class E<T> { foo: T }
}
declare class D4 extends N.C { }
