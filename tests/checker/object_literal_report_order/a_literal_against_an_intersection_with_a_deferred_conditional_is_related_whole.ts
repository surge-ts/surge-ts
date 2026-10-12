type Something<T> = { test: string } & (T extends object ? { arg: T } : { arg?: undefined });

function assigned<A extends object>(a: A, sa: Something<A>) {
    sa = { test: 'hi', arg: a };
    sa = { test: 'bye', arg: a, arr: a };
}

function declared<A extends object>(a: A) {
    const wrongMember: Something<A> = { test: 1, arg: a };
    const written: { test: string } & (A extends object ? { arg: A } : { arg?: undefined }) = { test: 'hi', arg: a };
}
