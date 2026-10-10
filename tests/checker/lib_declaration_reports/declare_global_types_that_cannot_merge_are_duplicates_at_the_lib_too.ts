// @lib: es2023,dom
// A module's `declare global` merges into the same globals.
export {};
declare global {
    type PropertyKey = string;
    interface ElementTagNameMap {
        x: number;
    }
}
