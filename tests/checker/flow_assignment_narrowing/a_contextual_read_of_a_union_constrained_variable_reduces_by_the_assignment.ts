function direct<T extends string | undefined>(x: T, y: NonNullable<T>) {
    x = y;
    y = x;
    let s1: string = x;
    let s2: string | number = x;
    const n: number = x;
}
function indexed<T extends { x: string | undefined }>(x: T["x"], y: NonNullable<T["x"]>) {
    x = y;
    y = x;
    let s1: string = x;
}
function branch<T extends string | undefined>(x: T, y: NonNullable<T>, c: boolean) {
    if (c) x = y;
    let s1: string = x;
}
function unassigned<T extends string | undefined>(x: T) {
    let s0: string = x;
}
function returned<T extends string | undefined>(x: T, y: NonNullable<T>): string {
    x = y;
    return x;
}
