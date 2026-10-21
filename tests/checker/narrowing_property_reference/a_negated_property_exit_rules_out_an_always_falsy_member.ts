// @strict: true
// After `if (!value.handler) throw`, a member whose `handler` is always falsy
// is gone (tsc's discriminant truthiness narrowing); one where it may be set
// either way stays.
interface WithHandler { handler: (x: number) => void; name: 'handled' }
interface WithoutHandler { handler?: undefined; name: 'plain' }
interface Undefined { handler: undefined; name: 'plain' }
interface MaybeA { handler?: () => void; name: 'a' }
interface MaybeB { handler?: () => void; name: 'b' }

export function optionalUndefined(value: WithHandler | WithoutHandler): 'handled' {
  if (!value.handler) { throw new Error(); }
  const name: 'handled' = value.name;
  value.handler(1);
  return name;
}

export function requiredUndefined(value: WithHandler | Undefined): 'handled' {
  if (!value.handler) { return 'handled'; }
  const name: 'handled' = value.name;
  return name;
}

export function bothMaybeSet(value: MaybeA | MaybeB): 'a' {
  if (!value.handler) { throw new Error(); }
  const name: 'a' = value.name;
  return name;
}
