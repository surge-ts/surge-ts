// @strict: true
type ComputedOf<T> = { [K in keyof T]: () => T[K] };
declare function computed<C>(members: ComputedOf<C>): C;
const both = computed({ count(): number { return 1; }, label: () => "x" });
const asString: string = both;
const mismatched = computed({ count: () => 1, plain: 42 });
declare function options<P, C>(options: { props: P; computed: ComputedOf<C> } & ThisType<P & C>): C;
const viaIntersection = options({ props: { x: 10 }, computed: { total(): number { return 42; }, wrong: 42 } });
const fromIntersection: string = options({ props: {}, computed: { name: () => "n" } });
type Boxed<T> = { [K in keyof T]: { value: T[K] } };
declare function unbox<T>(boxed: Boxed<T>): T;
const unboxed = unbox({ a: { value: 1 }, b: { value: "s" } });
const unboxedString: string = unboxed;
