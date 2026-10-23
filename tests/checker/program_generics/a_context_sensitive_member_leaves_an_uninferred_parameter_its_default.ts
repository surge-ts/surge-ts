// @strict: true
// A member whose parameters are all unannotated is skipped by the first
// inference pass; the second fixes what its contextual parameter types name
// first, so a parameter only it mentions takes its default and the member's
// parameter is typed from it, not left an implicit `any`.
interface Source<R> { pull?(c: Ctl<R>): void }
interface Ctl<R> { enqueue(chunk: R): void }
declare class Stream<R = any> { constructor(source: Source<R>); r: R }
declare class NoDefault<R> { constructor(source: Source<R>); r: R }
declare function make<R = any>(source: Source<R>): Stream<R>;
class Local<R> { constructor(source: Source<R>) {} }

export const a = new Stream({ pull(c) { c.enqueue(1); } });
export const b = new NoDefault({ pull(c) { c.enqueue(1); } });
export const c = make({ pull(c) { c.enqueue(1); } });
export const f = new Local({ pull: (c) => { c.enqueue(1); } });
export const br: string = b.r;

// Its return still infers.
interface Producer<R> { make(n: number): R; seed?: number }
declare function produce<R = string>(p: Producer<R>): R;
declare class Factory<R = string> { constructor(p: Producer<R>); value: R }
export const p1: boolean = produce({ make(n) { return n; } });
export const p2: boolean = produce({ make: (n) => [n] });
export const p3: boolean = new Factory({ make(n) { return n > 0; } }).value;
export const p4: string = new Factory({ make(n) { return n; } }).value;
