// @strict: true
export {}
interface N0 { nodeType: number }
interface E1 extends N0 { a: number }
interface E2 extends N0 { b: number }
interface Map1 { x: E1; y: E2 }
declare function isB<T extends keyof Map1, U extends Map1[T]>(node: N0 | null, t: T): node is U;
export function g<T extends keyof Map1>(node: N0 | null, t: T) {
    if (isB(node, t)) {
        node.nodeType;
    }
}
