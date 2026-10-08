interface Id { kind: "id"; name: string }
interface Obj { kind: "obj"; props: string[] }
interface Arr { kind: "arr"; elements?: (Id | Obj | null)[] }
interface Decl { id: Id | Arr }
declare const j: { ArrayPattern: { check(v: any): v is Arr }; ObjectPattern: { check(v: any): v is Obj } };
declare function take(o: Obj): void;
export function f(declarator: Decl | null, tuple: Arr | null) {
  const deep = j.ArrayPattern.check(declarator?.id) && j.ObjectPattern.check(tuple?.elements?.[0]) ? tuple?.elements?.[0] : null;
  if (deep) take(deep);
  const deep2 = j.ObjectPattern.check(tuple?.elements?.[0]) ? tuple?.elements?.[0] : null;
  if (deep2) take(deep2);
}
