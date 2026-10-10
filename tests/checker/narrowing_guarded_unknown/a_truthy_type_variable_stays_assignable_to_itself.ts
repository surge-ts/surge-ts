// A truthy `T` is `T & {}`, still assignable to `T`.
function lazy<T>(getter: () => T): { value: T } {
    let cached: T;
    return {
        get value() {
            if (cached) return cached;
            const value = getter();
            cached = value;
            return value;
        },
    };
}
function first<E>(error: E | null): E | undefined {
    if (error) {
        const narrowed: E = error;
        return narrowed;
    }
    return undefined;
}
