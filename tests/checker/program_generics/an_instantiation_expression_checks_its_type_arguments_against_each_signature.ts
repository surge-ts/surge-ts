declare const g2: {
    <T extends string>(a: T): T;
    new <T extends number>(b: T): T;
}
declare function g4<T extends string>(a: T): T;
declare function g5<T extends string>(a: T): T;
declare function g5<T extends number, U>(a: T, b: U): T;

const x1 = g2<number>;
const x2 = g4<number>;
const x3 = g4<"ok">;
const x4 = g5<boolean>;

type T40<U extends string> = typeof g2<U>;
type T41<U extends number> = typeof g2<U>;
type T42 = typeof g4<number>;
type T43<U extends number> = typeof g4<U>;
type T44<U extends "a" | "b"> = typeof g4<U>;

function inBody<V extends number>() {
    const y = g4<V>;
}
