// A `for…in` over a type variable binds `Extract<keyof T, string>`
// (`getExtractStringType(getIndexType(T))`), which relates as that type.
function readKeys<T>(t: T) {
    for (const key in t) {
        const n: number = key;
    }
}
function constrained<T, K extends Extract<keyof T, string>>(t: T, k: K, tk: T[K]): void {
    for (let key in t) {
        key = k;
        k = key;
        t[key] = tk;
        tk = t[key];
    }
}
function copy<T extends object>(o: T): T {
    const result = {} as T;
    for (const key in o) {
        result[key] = o[key];
    }
    return result;
}
function sameValues<T>(a: T, b: T) {
    for (const key in a) {
        if (a[key] !== b[key]) return false;
    }
    return true;
}
