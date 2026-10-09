// @strict: true
interface Node<T> { next: AnyNode; value: T }
type AnyNode = Node<any>;
declare const node: AnyNode;
export const whole: 0 = node;
export const member: 0 = node.next;
type LazyLoader<T> = { load: () => Promise<void>; ref: () => Promise<T> };
interface RouterDef<TRecord> { router: true; record: TRecord; lazy: Record<string, LazyLoader<AnyRouter>> }
interface Router<TRecord> { _def: RouterDef<TRecord> }
type AnyRouter = Router<any>;
interface Procedure { (input: number): void; _def: { procedure: true } }
declare function isRouter(value: unknown): value is AnyRouter;
declare const item: AnyRouter | Procedure;
if (isRouter(item)) {
    const record: 0 = item._def.record;
} else {
    const procedure: 0 = item;
}
