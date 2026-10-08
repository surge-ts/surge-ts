class Rel { kind = "rel" }
class One extends Rel { config?: { references: string[]; fields: string[] } }
declare function is<T extends abstract new (...args: any) => any>(value: any, type: T): value is InstanceType<T>;
export function a(xs: Rel[]) {
  if (is(xs[0], One) && xs[0].config) { return xs[0].config.references; }
  return null;
}
export function b(xs: One[]) {
  if (xs[0].config) { return xs[0].config.references; }
  return null;
}
export function c(xs: Rel[]) {
  if (is(xs[0], One)) { if (xs[0].config) { return xs[0].config.references; } }
  return null;
}
export function d(xs: One[]) {
  if (xs[0] && xs[0].config) { return xs[0].config.references; }
  return null;
}
export function e(reverseRelations: Rel[]) {
  if (reverseRelations[0] && is(reverseRelations[0], One) && reverseRelations[0].config) {
    return { fields: reverseRelations[0].config.references };
  }
  if (is(reverseRelations[0], One)) {
    return reverseRelations[0].config.references;
  }
  return null;
}
