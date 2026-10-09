// @strict: true
// @target: es2020
declare function last_indexes(a: number): void;
declare function last_indexes(a: ArrayLike<bigint>): void;
last_indexes([1, 2, 3]);

declare function last_is_scalar(a: ArrayLike<bigint>): void;
declare function last_is_scalar(a: number): void;
last_is_scalar([1, 2, 3]);

declare function both_index(a: ArrayLike<bigint>): void;
declare function both_index(a: bigint[]): void;
both_index([1, 2, 3]);

declare function property(a: { x: number }): void;
declare function property(a: { y: number }): void;
property({ x: "s", y: "t" });

declare function member(a: number): void;
declare function member(a: { y: number; z: string }): void;
member({ y: 1, z: 2, w: 3 });

declare function callback(a: number): void;
declare function callback(a: (x: number) => string): void;
callback((x) => x);

new BigInt64Array([1, 2, 3]);
