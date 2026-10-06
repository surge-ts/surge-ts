// @module: commonjs
// @filename: modules.d.ts
declare module "interface" {
    interface Foo {
        x: number;
    }
    export = Foo;
}
declare module "interface-module" {
    interface Foo {
        x: number;
    }
    namespace Foo {
        export var a: number;
    }
    export = Foo;
}
// @filename: main.ts
/// <reference path="modules.d.ts"/>
import * as y1 from "interface";
import * as y2 from "interface-module";
y1.a;
y2.a;
let v: y1 = { x: 1 };
