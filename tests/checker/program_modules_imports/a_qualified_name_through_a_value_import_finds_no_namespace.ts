// @filename: a.ts
export default function MyFn() {}
export function named() {}
export class Klass {}
export namespace Space { export interface T {} }
export function both() {}
export namespace both { export interface T {} }

// @filename: b.ts
import MyFn, { named, Klass, Space, both } from './a';
let t1: MyFn.bar;
let t2: named.q;
let t3: Klass.x;
let t4: Space.T;
let t5: both.T;
