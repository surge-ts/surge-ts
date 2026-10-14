// @strict: true
// `MapConstructor`'s first signature is `new (): Map<any, any>`, wherever the
// expression sits.
interface DescriptionMap {
  properties: Map<string, string>;
}
export function reduced(): void {
  let d: DescriptionMap | null = null;
  d ??= { properties: new Map() };
  const s: string = d;
}
const o = { p: new Map() };
export const y: { p: Map<string, string> } = o;
const arr = [new Map()];
export const n: number = arr;
