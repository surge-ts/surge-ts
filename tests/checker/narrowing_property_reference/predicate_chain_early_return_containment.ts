interface Call { kind: "call"; callee: Id | Call }
interface Id { kind: "id"; name: string }
interface Decl { init: Call | number | null; id: Id | Call }
declare const j: { CallExpression: { check(value: any): value is Call }; Identifier: { check(value: any): value is Id } };
export function f(declarator: Decl | null) {
  if (!j.CallExpression.check(declarator?.init) || !j.Identifier.check(declarator.init.callee) || !['a', 'b'].includes(declarator.init.callee.name)) {
    return;
  }
  declarator.id = { kind: "id", name: "" };
}
export function g(declarator: Decl | null) {
  if (!j.CallExpression.check(declarator?.init) || !j.Identifier.check(declarator.init.callee)) {
    return;
  }
  declarator.id = { kind: "id", name: "" };
}
export function h(declarator: Decl | null) {
  if (!j.CallExpression.check(declarator?.init)) {
    return;
  }
  declarator.id = { kind: "id", name: "" };
}
export function bad(declarator: Decl | null) {
  if (j.CallExpression.check(declarator?.init) || j.Identifier.check(declarator?.id)) {
    return;
  }
  return declarator.init;
}
