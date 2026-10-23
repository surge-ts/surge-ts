// @strict: true
// `node.init?.type === "id"` holding proves `node.init` present, which rules
// out a union member whose `init` is always `null`: `node.init` is the
// identifier, not a degraded read.
interface Identifier { type: "id"; name: string }
interface Literal { type: "lit"; value: string }
type Expression = Identifier | Literal;
interface MaybeInit { init: Expression | null; definite: false }
interface NoInit { init: null; definite: false }
type Declarator = MaybeInit | NoInit;

export function viaOptionalChain(node: Declarator) {
  if (node.init?.type === "id") {
    const name: number = node.init.name;
  }
}

export function viaExplicitCheck(node: Declarator) {
  if (node.init !== null && node.init.type === "id") {
    const name: number = node.init.name;
  }
}
