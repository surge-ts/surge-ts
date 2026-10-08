interface Call { kind: "call"; callee: string }
interface Decl { init: Call | number | null }
declare const t: { check(value: any): value is Call };
declare function isCall(value: any): value is Call;
export function f(declarator: Decl | null) {
  if (!isCall(declarator?.init) || declarator.init.callee) { return; }
  if (!t.check(declarator?.init) || declarator.init.callee) { return; }
}
export function g(declarator: Decl | null) {
  if (!t.check(declarator?.init)) { return; }
  const c: string = declarator.init.callee;
}
export function h(declarator: Decl | null) {
  const ok = isCall(declarator?.init) && declarator.init.callee.length > 0;
  const bad = isCall(declarator?.init) || declarator.init;
}
