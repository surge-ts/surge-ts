// @strict: true
// @lib: es2023,dom
// A generic call's own type parameter is no contextual type for choosing a
// constructor overload: `new Uint32Array(2)` is the length overload.
export const words = crypto.getRandomValues(new Uint32Array(2));
declare function view<T extends ArrayBufferView<ArrayBuffer> | null>(array: T): T;
export const bytes = view(new Uint8Array([1, 2]));
export const wrong = view(new Uint8Array("x"));
export const annotated: Uint32Array<ArrayBufferLike> = new Uint32Array(2);
export const fromArray: Uint32Array<ArrayBuffer> = new Uint32Array([1]);
