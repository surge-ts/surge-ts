type SetStateInternal<T> = { _(partial: T | Partial<T>, replace?: false): void }['_'];
interface StoreApi<T> { setState: SetStateInternal<T>; getState: () => T }
type Get<T, K, F> = K extends keyof T ? T[K] : F;
interface StoreMutators<S, A> {}
type StoreMutatorIdentifier = keyof StoreMutators<unknown, unknown>;
type Mutate<S, Ms> = number extends Ms['length' & keyof Ms]
  ? S
  : Ms extends []
    ? S
    : Ms extends [[infer Mi, infer Ma], ...infer Mrs]
      ? Mutate<StoreMutators<S, Ma>[Mi & StoreMutatorIdentifier], Mrs>
      : never;
type StateCreator<T, Mis extends [StoreMutatorIdentifier, unknown][] = []> = (
  setState: Get<Mutate<StoreApi<T>, Mis>, 'setState', never>,
  getState: () => T,
) => T;
interface State { count: number; increment: () => void }
const foo: <M extends [StoreMutatorIdentifier, unknown][]>() => StateCreator<State, M> = () => (set, get) => ({
  count: 0,
  increment: () => { set({ count: get().count + 1 }) },
});
function g<M extends [StoreMutatorIdentifier, unknown][]>(set: Get<Mutate<StoreApi<State>, M>, 'setState', never>) {
