// @strict: true
export namespace C { export interface I { a: number } }
export import v = C;
export namespace M {
    export namespace C { export interface I { } }
    export var w: v.I;
}
