// A conditional source keeps a failed branch beneath its default constraint
// against a like conditional, and relates through its distributive
// constraint only to a target that is no conditional.
// @surge-compare: messages
type Grow1<T extends unknown[], N extends number> = T['length'] extends N ? T : Grow1<[number, ...T], N>;
type Grow2<T extends unknown[], N extends number> = T['length'] extends N ? T : Grow2<[string, ...T], N>;
function grow<T extends number>(x: Grow1<[], T>, y: Grow2<[], T>) {
    x = y;
}

type TupleOf<T, N extends number> = N extends N ? number extends N ? T[] : _TupleOf<T, N, []> : never;
type _TupleOf<T, N extends number, R extends unknown[]> = R['length'] extends N ? R : _TupleOf<T, N, [T, ...R]>;
function tuples<N extends number, M extends N>(tn: TupleOf<number, N>, tm: TupleOf<number, M>) {
    tn = tm;
    tm = tn;
}

type Exact<T> = T extends string ? true : 42;
type Loose<T> = T extends string ? boolean : number;
const narrow = <U>(value: Loose<U>): Exact<U> => value;

type Pick1<T> = T extends string ? 1 : 2;
function distributive<T extends string>(c: Pick1<T>) {
    let n: 3 = c;
}
type Same<T> = T extends string ? [T] : 2;
function sameExtends<T extends string>(c: Same<T>, d: Pick1<T>) {
    d = c;
}
export { grow, tuples, narrow, distributive, sameExtends };
