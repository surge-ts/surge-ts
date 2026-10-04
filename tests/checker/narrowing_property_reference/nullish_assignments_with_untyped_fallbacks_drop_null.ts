// @target: es2020
function m1<TReturn>(fn: () => Promise<TReturn>) {
    let promise: Promise<TReturn> | null = null;
    return async () => {
        promise ??= fn().catch((cause) => { throw cause; });
        const v: TReturn = await promise;
        promise = null;
        return v;
    };
}
