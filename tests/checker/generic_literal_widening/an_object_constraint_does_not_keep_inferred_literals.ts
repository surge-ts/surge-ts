class C<T extends { length: number }> {
    constructor(x: T) { }
    foo<U extends T>(x: U) { return x; }
}
var c = new C({ length: 2 });
const n: string = c;
declare function mk<T extends { length: number }>(x: T): T;
const m = mk({ length: 2 }); const n2: string = m;
