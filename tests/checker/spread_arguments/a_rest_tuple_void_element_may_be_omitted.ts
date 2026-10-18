// `getMinArgumentCount` walks the positions below a rest tuple's minimum back
// while they accept `void`, so tanstack's `UseMutateFunction` over
// `Parameters<…>` with `void` variables takes no argument.
type MutateFunction<V> = (variables: V, options?: { retry: number }) => void;
type UseMutateFunction<V> = (...args: Parameters<MutateFunction<V>>) => void;
declare const mutateVoid: UseMutateFunction<void>;
mutateVoid();
declare const mutateString: UseMutateFunction<string>;
mutateString();
declare const written: (...args: [a: void, b?: number]) => void;
written();
declare const voidLast: (...args: [a: number, b: void]) => void;
voidLast(1);
voidLast();
