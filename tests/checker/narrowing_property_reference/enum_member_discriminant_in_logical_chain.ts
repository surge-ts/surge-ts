// @filename: ast.ts
export declare enum T { Id = "Id", Lit = "Lit", Prop = "Prop", Arrow = "Arrow", Fn = "Fn" }
// @filename: main.ts
import * as AST from "./ast";
import { T } from "./ast";
interface Id { type: T.Id; name: string }
interface Lit { type: T.Lit; value: number }
interface Arrow { type: T.Arrow; body: number }
interface Fn { type: T.Fn; body: string }
type Expr = Id | Lit | Arrow | Fn;
interface PropComputed { type: T.Prop; key: Expr; computed: true }
interface PropNon { type: T.Prop; key: Id; computed: false }
type Prop = PropComputed | PropNon;
interface DeclA { kind: "a"; init: Expr | null }
interface DeclB { kind: "b"; init: Expr }
export function f(p: Prop, node: DeclA | DeclB) {
  if (p.type === T.Prop && p.key.type === T.Id) {
    const name: string = p.key.name;
  }
  if (p.type === AST.T.Prop && p.key.type === AST.T.Id && p.key.name) {
    const name: string = p.key.name;
  }
  if (node.init !== null && (node.init.type === T.Arrow || node.init.type === T.Fn)) {
    const body: string | number = node.init.body;
    const wrong: 1 = node.init.body;
  }
}
