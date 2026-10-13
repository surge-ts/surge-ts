// A homomorphic mapping (`Writable<T>`) reverse-maps a readonly tuple to
// itself, readonly-ness included, so `T | Writable<T>` takes an `as const`
// tuple; an inference through a mapping is secondary to a direct one.
type Writable<T> = { -readonly [P in keyof T]: T[P] };
interface Init<N, T> {
    name: N;
    values: T;
}
declare function enumOf<U extends string, T extends Readonly<[U, ...U[]]>>(values: T | Writable<T>): Init<"", Writable<T>>;
declare function enumOf<TName extends string, U extends string, T extends Readonly<[U, ...U[]]>>(
    name: TName,
    values: T | Writable<T>,
): Init<TName, Writable<T>>;
const named = enumOf("test", ["a", "b", "c"] as const);
const plain = enumOf(["a", "b"] as const);
const wrong: string = named;
declare function objAndPartial<T>(primary: T, secondary: Partial<T>): T;
objAndPartial({ x: 0, y: 0 }, { x: 1, y: 1, z: 1 });
