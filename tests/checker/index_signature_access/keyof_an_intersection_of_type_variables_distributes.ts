// @strict: true
function f<T, U>(y: T & U, k3: keyof (T & U), k4: keyof T | keyof U) {
    k3 = k4;
    k4 = k3;
    y[k4];
}
