// @filename: file.ts
export class Foo {
    member!: string;
}
export interface Shape {
    a: string;
}

// @filename: reexport.ts
export * from "./file";

// @filename: augment.ts
import * as ns from "./reexport";
declare module "./reexport" {
    export enum Foo {
        A, B, C
    }
    export interface Shape {
        b: number;
    }
}
declare const shape: ns.Shape;
const b: string = shape.b;
