// @declaration: true
// @strict: true
// @module: commonjs
// @filename: subs.ts
interface Base<E> { error: E | null }
interface Idle extends Base<null> { state: "idle" }
interface Busy<E> extends Base<E> { state: "busy" }
interface Secret { s: string }
export type State<E> = Idle | Busy<E>;
type HiddenState<E> = Idle | Busy<E>;
export type Plain = Idle | Secret;
export interface Observer<E> { onState: (state: State<E>) => void }
export declare function create<E>(): { observe(observer: Partial<Observer<E>>): void; watch(cb: (state: State<E>) => void): void };
export declare function hidden<E>(): { watch(cb: (state: HiddenState<E>) => void): void };
export declare function secretArgument(): { watch(cb: (state: State<Secret>) => void): void };
export declare function plain(): { watch(cb: (state: Plain) => void): void };
// @filename: a.ts
import { create, hidden, secretArgument, plain } from "./subs";
export const a = create<string>();
export const h = hidden<string>();
export const s = secretArgument();
export const p = plain();
// @filename: b.ts
import { create, hidden, secretArgument, plain } from "./subs";
export const b = create<string>();
export const h2 = hidden<string>();
export const s2 = secretArgument();
export const p2 = plain();
