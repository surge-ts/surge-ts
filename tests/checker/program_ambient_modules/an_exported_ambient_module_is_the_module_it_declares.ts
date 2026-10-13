// @module: commonjs
export namespace N { export class c1 {} }
export declare module "m2" {
    import b = N;
    export var z: b;
    export var u: b.c1;
}
