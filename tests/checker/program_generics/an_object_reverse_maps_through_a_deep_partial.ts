// A homomorphic mapping whose template reads the source member as one branch
// (`DeepPartial<T>`) reverse-maps an object member for member, so `T` infers
// from it when nothing else does.
type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };
declare function useForm<T>(options: { defaultValues?: DeepPartial<T> }): { register(key: keyof T): void };
useForm({ defaultValues: { name: "x" } }).register("image");
type GuardedDeepPartial<T> = T extends Date ? T : { [K in keyof T]?: T[K] extends object ? GuardedDeepPartial<T[K]> : T[K] };
declare function useGuarded<T>(options: { defaultValues?: GuardedDeepPartial<T> }): { register(key: keyof T): void };
useGuarded({ defaultValues: { name: "x" } }).register("image");
