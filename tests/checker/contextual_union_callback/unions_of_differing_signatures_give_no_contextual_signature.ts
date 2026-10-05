// @noImplicitAny: true
var b6: ((s: string, w: boolean) => void) | ((n: number) => number);
var b7: ((s: string, w: boolean) => void) | ((s: string, w: number) => string);
b6 = (k) => { k.toLowerCase() };
b7 = (j, m) => { };
var b8: ((s: string) => void) | ((s: string) => number);
b8 = (q) => { q.toLowerCase() };
class C<T, U> {
    constructor() {
        var k: ((j: T, k: U) => (T|U)[]) | ((j: number,k :U) => number[]) = (j, k) => {
            return [j, k];
        }
    }
}
