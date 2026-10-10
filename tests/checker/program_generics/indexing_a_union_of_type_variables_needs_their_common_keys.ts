// Indexing `T | U` takes a key of both: `keyof (T | U)` is `keyof T & keyof U`
// (`checkIndexedAccessIndexType`).
function f<T, U>(x: T | U, k1: keyof (T | U), k2: keyof T & keyof U, k3: keyof (T & U), k4: keyof T | keyof U) {
    x[k1];
    x[k2];
    x[k3];
    x[k4];
}
