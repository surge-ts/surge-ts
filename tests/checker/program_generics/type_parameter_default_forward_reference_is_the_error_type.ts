// @filename: decl.ts
interface Lifecycle<S> { state?: S }
interface Vnode<S = Lifecycle<S>> { tag: S; }
interface Component { view(vnode: Vnode): number; }
interface Pair<A = B, B = string> { a: A; b: B }
// @filename: use.ts
declare const component: Component;
declare const pair: Pair;
const n: number = component.view({ tag: {} });
const s: string = pair.b;
