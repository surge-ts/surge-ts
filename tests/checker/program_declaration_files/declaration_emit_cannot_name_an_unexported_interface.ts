// @declaration: true
// @strict: true
// @module: commonjs
// @filename: a.ts
interface Hidden { x: number }
type HiddenAlias = { y: number };
class HiddenClass { k = 1 }
enum HiddenEnum { A }
type Symbolic = { readonly key: unique symbol };
interface Renamed { r: number }
export { Renamed as Visible };
export interface Shown { s: number }
export function hidden(): Hidden { return null!; }
export function alias(): HiddenAlias { return null!; }
export function instance(): HiddenClass { return null!; }
export function member(): HiddenEnum { return HiddenEnum.A; }
export const symbolic: Symbolic = null!;
export function list(): Array<Hidden> { return []; }
export function renamed(): Renamed { return null!; }
export function shown(): Shown { return null!; }
// @filename: ambient.d.ts
interface Implicit { i: number }
export declare function implicit(): Implicit;
// @filename: b.ts
import { hidden, alias, instance, member, symbolic, list, renamed, shown } from "./a";
import { implicit } from "./ambient";
export const fromHidden = hidden();
export const fromAlias = alias();
export const fromInstance = instance();
export const fromEnum = member();
export const fromSymbolic = symbolic;
export const fromList = list();
export const fromRenamed = renamed();
export const fromShown = shown();
export const fromImplicit = implicit();
export function returned() { return hidden(); }
export function expando() {}
expando.value = hidden();
expando.value = hidden();
export const arrow = () => hidden();
export const nested = { inner: hidden() };
const local = hidden();
export { local };
const unexportedLocal = hidden();
export default hidden();
