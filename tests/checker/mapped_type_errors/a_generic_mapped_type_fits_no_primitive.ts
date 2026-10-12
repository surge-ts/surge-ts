declare function takeNum(n: number): void;

function read<T>(partial: Partial<T>, picked: Pick<T, keyof T>, omitted: Omit<T, "x">) {
    const n: number = partial;
    let s: string;
    s = picked;
    takeNum(omitted);
    const either: string | number = partial;
    const kept: {} = partial;
    const same: Partial<T> = partial;
}

function arrayLike<T extends unknown[]>(partial: Partial<T>) {
    const n: number = partial;
}
