function f4<T extends any[]>(t: T) {
    function f(cb: (x: number, ...args: T) => void) {}
    f((...x) => {});
    f((a, ...x) => {});
    f((a, b, ...x) => {});
    const g: (x: number, ...args: T) => void = (a, b, ...x) => {};
    f((a: number, b: T[0], ...x: T[number][]) => {});
    f((a, b) => {});
    f((a, b, ...x) => { const s: boolean = b; const u: boolean = x; });
    f((a, ...x) => { const v: boolean = x; });
}
