// `!isNode(n) || isBar(n)` evaluates its right operand only when `isNode(n)`
// held, so the argument there is a `Node`, in an `if` condition as anywhere.
type FooNode = { kind: "foo"; children: Node[] };
type BarNode = { kind: "bar" };
type Node = FooNode | BarNode;
type Doc = { kind: "document"; children: Node[] };
declare function isNode(node: unknown): node is Node;
declare function isBar(node: Node): node is BarNode;
export function negated(node: Doc | Node) {
    if (!isNode(node) || !isBar(node)) {
        return;
    }
}
export function plain(node: Doc | Node) {
    if (!isNode(node) || isBar(node)) {
        return;
    }
}
export function unguarded(node: Doc | Node) {
    if (isBar(node) || !isNode(node)) {
        return;
    }
}
