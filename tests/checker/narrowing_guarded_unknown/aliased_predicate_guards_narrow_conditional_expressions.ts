declare function isArr(v: unknown): v is unknown[];
function f(a: unknown[] | Record<string, unknown>, b: unknown[] | Record<string, unknown>) {
    const ok = isArr(b);
    const z = ok ? b : 1;
    const n3: string = z;
    const both = isArr(a) && isArr(b);
    const w = both ? b.length : 0;
    const n4: string = w;
}
function g(a: any, b: any) {
    const array = isArr(a) && isArr(b);
    const bItems = array ? b : Object.keys(b);
    const size: number = bItems.length;
}
