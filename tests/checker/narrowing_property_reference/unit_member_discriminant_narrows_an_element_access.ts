// @strict: true
// @noUncheckedIndexedAccess: true
// @lib: es2023
enum Kind { Arr = "ArrayExpression", Id = "Identifier" }
type ArrayNode = { type: Kind.Arr; elements: number[] };
type IdentifierNode = { type: Kind.Id; name: string };
declare const node: { arguments: (ArrayNode | IdentifierNode)[] };
declare const hook: string | undefined;
declare const sink: { push(item: { deps: ArrayNode }): void };
if (hook !== undefined && node.arguments.length > 1 && node.arguments[1]?.type === Kind.Arr) {
  sink.push({ deps: node.arguments[1] });
}
if (node.arguments[0] && node.arguments[0].type === Kind.Arr) {
  const first: ArrayNode = node.arguments[0];
}
if (node.arguments[0]?.type === Kind.Id) {
  const wrong: ArrayNode = node.arguments[0];
}
