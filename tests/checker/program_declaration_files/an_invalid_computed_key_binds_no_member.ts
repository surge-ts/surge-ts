// @declaration: true
// @strict: true
// @filename: type.ts
// A computed key that is no property key (an enum object) binds no member, so
// declaration emit has no late-bound name to write.
export namespace Foo {
    export enum Enum {
        A = "a",
    }
}
export type Invalid = { x?: { [Foo.Enum]: 0 } };
// @filename: index.ts
import { type Invalid } from "./type";
export const invalid = { ...({} as Invalid) };
