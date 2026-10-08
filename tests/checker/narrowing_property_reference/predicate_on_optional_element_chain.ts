interface Call { kind: "call"; callee: string }
interface Id { kind: "id"; name: string }
interface Decl { init: Call | number | null; id: Id | Call; elements?: (Id | null)[] }
declare const t: { check(value: any): value is Call };
declare const ti: { check(value: any): value is Id };
declare const ta: { check(value: any): value is Decl };
export function f(declarator: Decl | null) {
  if (!t.check(declarator?.init) || !declarator.init.callee) { return; }
  declarator.id = { kind: "id", name: "" };
}
export function g(tuple: Decl | null) {
  const a = ti.check(tuple?.elements?.[0]) ? tuple.elements[0].name : null;
}
export function h(tuple: Decl | null) {
  const bad = ti.check(tuple?.elements?.[0]) ? null : tuple.elements;
}
