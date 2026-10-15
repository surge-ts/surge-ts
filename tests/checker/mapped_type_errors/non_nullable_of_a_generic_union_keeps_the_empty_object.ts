// @strict: true
function g1<T>(x: T[keyof T], y: NonNullable<T[keyof T] | undefined>) { y = x; }
function g2<T>(x: Partial<T>[keyof T], y: NonNullable<Partial<T>[keyof T]>) {
    x = y;
    y = x;
}
function g3<T>(x: T | null, y: NonNullable<T | null>) {
    x = y;
    y = x;
}
