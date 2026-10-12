type Assign<T, U> = Omit<T, keyof U> & U;

function omitted<T>(t: T) {
    const o = { x: 1 };
    const kept: Omit<T, "x"> = o;
    const assigned: Assign<T, { x: number }> = o;
}

function keyed<T>(t: T, ks: keyof (T & string)) {
    const k: keyof string = "padStart";
    const n: number = ks;
}
