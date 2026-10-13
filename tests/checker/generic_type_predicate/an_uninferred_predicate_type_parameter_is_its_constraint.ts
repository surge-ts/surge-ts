interface K { k: number }
declare function isK<U extends K>(node: K | null): node is U;
function g1(node: K | null) {
    if (isK(node)) {
        const n: K = node;
    }
}
interface M { a: { k: 1 }; b: { k: 2 } }
declare function isM<T extends keyof M, U extends M[T]>(node: { k: number } | null, tag: T): node is U;
function g2(node: { k: number } | null) {
    if (isM(node, "a")) {
        const n: { k: number } = node;
    }
}
