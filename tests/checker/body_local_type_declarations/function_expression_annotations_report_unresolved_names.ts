const o2 = { p: function (x: V2): R2 { return x; } };
const o3 = { p: (x: V3) => x };
declare function take(callback: any): void;
take({ p: (x: V4) => 0 });
take((x: V5, y: number): R5 => x);
const a6 = [(x: V6) => 0];
function generic<T>() {
    return { p: (x: T, y: U7) => x };
}
