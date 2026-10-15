// @strict: true
function f<T>(x: T, k: keyof (T & {}), y: T & {}) {
    x[k];
    const narrower: keyof T = k;
    y[k];
}
function g<T>(t: T & {}, k: keyof T) {
    t[k];
}
