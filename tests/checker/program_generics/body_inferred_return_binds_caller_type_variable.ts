function f1<T>(a: (x: T) => T) { var r!: (x: T) => T; return r; }
function f3<T>(a: T) { return a; }
function f4<T>(a: T) { var r!: T[]; return r; }
function other<U extends Date>(x: U) {
    const z1: string = f1((a: U) => a);
    const z3: string = f3(x);
    const z4: string = f4(x);
    f1((a: U) => a)(1);
}
