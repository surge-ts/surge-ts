// @isolatedModules: true
// @module: esnext
namespace Types {
    export type A = string;
}
export import T = Types;
namespace Values {
    export const v = 1;
}
export import V = Values;
