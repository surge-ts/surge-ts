type DistributedKeyOf<T> = T extends unknown ? keyof T : never;
type NarrowByKeyValue<ObjT, KeyT extends PropertyKey, ValueT> = ObjT extends unknown
    ? KeyT extends keyof ObjT
        ? ValueT extends ObjT[KeyT]
            ? ObjT & Readonly<Record<KeyT, ValueT>>
            : never
        : never
    : never;
type NarrowByDeepValue<ObjT, DeepPathT, ValueT> = DeepPathT extends readonly [
    infer Head extends DistributedKeyOf<ObjT>,
]
    ? NarrowByKeyValue<ObjT, Head, ValueT>
    : DeepPathT extends readonly [infer Head extends DistributedKeyOf<ObjT>, ...infer Rest]
    ? NarrowByKeyValue<ObjT, Head, NarrowByDeepValue<NonNullable<ObjT[Head]>, Rest, ValueT>>
    : never;
declare function satisfies<ObjT extends object, const DeepPathT extends ReadonlyArray<number | string>, ValueT>(
    obj: ObjT,
    deepPath: DeepPathT,
    predicate: (arg: unknown) => arg is ValueT,
): obj is NarrowByDeepValue<ObjT, DeepPathT, ValueT>;
type Foo = { value: { type: 'A' }; a?: number } | { value: { type: 'B' }; b?: number };
type FooA = { value: { type: 'A' }; a?: number };
declare function isA(arg: unknown): arg is 'A';
declare function isB(arg: unknown): arg is 'B';
declare function assert(condition: boolean): asserts condition;
function positive(foo: Foo): FooA {
    assert(satisfies(foo, ['value', 'type'], isA));
    return foo;
}
function negative(foo: Foo): FooA {
    assert(!satisfies(foo, ['value', 'type'], isB));
    return foo;
}
function wrong(foo: Foo): FooA {
    assert(satisfies(foo, ['value', 'type'], isB));
    return foo;
}
const deep: number = null! as NarrowByDeepValue<Foo, readonly ['value', 'type'], 'B'>;
