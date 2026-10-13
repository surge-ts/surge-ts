// @module: commonjs
// @target: es2015
export { }
declare global {
    import f = require("not-a-module");
    import { g } from "also-not-a-module";
}
