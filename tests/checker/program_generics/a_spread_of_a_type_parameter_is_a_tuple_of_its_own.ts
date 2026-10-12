function f11<T extends unknown[]>(t: T, m: [...T], r: readonly [...T]) {
    t = m;
    t = r;
    m = t;
    m = r;
    r = t;
    r = m;
}

function f12<T extends readonly unknown[]>(t: T, m: [...T], r: readonly [...T]) {
    t = m;
    t = r;
    m = t;
    m = r;
    r = t;
    r = m;
}

declare function concat<T extends unknown[], U extends unknown[]>(a: [...T], b: [...U]): [...T, ...U];
const joined: [number, number, string] = concat([1, 2], ["x"]);
const wrong: [number] = concat([1, 2], ["x"]);
