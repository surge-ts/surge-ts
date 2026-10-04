// @filename: SubModule.ts
export class c { }
// @filename: main.ts
declare module "OuterModule" {
    import m2 = require("./SubModule");
    import * as m3 from "./SubModule";
}
