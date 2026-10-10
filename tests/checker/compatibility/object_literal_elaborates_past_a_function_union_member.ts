// @strict: true
// @lib: es2023
type SetState<S> = (value: S | ((prev: S) => S)) => void;
declare const setUser: SetState<{ name: string } | null>;
setUser({ name: 1 });
setUser((prev) => prev);
setUser(null);
declare function update(value: { count: number } | ((prev: number) => number)): void;
update({ count: "x" });
update((prev) => prev + 1);
