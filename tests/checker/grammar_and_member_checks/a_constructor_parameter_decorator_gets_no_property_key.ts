// @experimentalDecorators: true
// @strict: true
// A legacy parameter decorator on a constructor parameter is called with
// `undefined` for its property key: a decorator that takes only a
// `string | symbol` key cannot be applied there (TS1239).
declare function dec(target: Object, key: string | symbol, index: number): void;
declare function decOpt(target: Object, key: string | symbol | undefined, index: number): void;
class K {
    constructor(@dec a: number, @decOpt b: number) {}
    m(@dec c: number) {}
}
