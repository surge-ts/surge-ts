declare module "Outer1" {
    import m1 = require("./Missing1");
    import m2 = require("missing2");
    import { x } from "./missing3";
    import { y } from "missing4";
    export = m1;
}

declare module "Inner" {
    export const v: number;
}

declare module "Outer2" {
    import inner = require("Inner");
    import { v } from "Inner";
    export = inner;
}
