// @surge-compare: messages
type Opt<T> = { [K in keyof T]?: T[K] };
type IsString<T> = T extends string ? true : false;
function f<T>(o: Opt<T>) {
    const a: number = o;
}
function g<T>(p: Partial<T>) {
    const b: number = p;
}
function h<T>() {
    const c: IsString<T> = 1;
}
