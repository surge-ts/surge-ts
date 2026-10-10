// @strict: true
// A declaration file the program writes is checked like any other source: an
// index signature keyed by a class or a non-key keyword is TS1268.
// @filename: types.d.ts
declare class Box<T> { value: T }
declare class Key { k: number }
declare var a: { [x: Box]: Box };
declare var b: { [x: Key]: number };
interface Flags { [x: boolean]: number }
interface Good { [x: string]: number; [y: number]: number }
