export const x: import(12) = undefined as any;
export function getFoo<T extends "./a" | "./b">(v: T): import(T).Foo {
    return undefined as any;
}
