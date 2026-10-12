interface Box<out A> { v: A }
declare const box: Box<unknown>;
const narrowed: Box<string> = box;
interface Parent<out A> { child: Child<A> | null; parent: Parent<A> | null; }
interface Child<A, B = unknown> extends Parent<A> { readonly a: A; readonly b: B; }
const pu: Parent<unknown> = { child: { a: 0, b: 0, child: null, parent: null }, parent: null };
const notString: Parent<string> = pu;
function read(p: Box<unknown>) {
    const inner: Box<string> = p;
    return inner;
}
