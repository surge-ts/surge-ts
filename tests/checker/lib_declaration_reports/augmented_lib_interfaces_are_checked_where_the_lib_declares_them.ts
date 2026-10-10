// @lib: es2023,dom
// Reopening a lib interface reaches the checks tsc runs on the lib's own
// declarations: its members against a new index signature (TS2411), its
// subinterfaces against the augmented base (TS2430), and an index signature
// the lib already declares (TS2374).
interface Object {
    [key: string]: Object;
}
interface Function {
    [n: number]: string;
}
interface HTMLElement {
    [index: number]: HTMLElement;
}
interface Array<T> {
    [n: number]: T;
}
declare const callback: () => void;
const asFunction: Function = callback;
