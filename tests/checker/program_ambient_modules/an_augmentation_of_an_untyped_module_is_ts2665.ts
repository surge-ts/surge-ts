// @filename: node_modules/js/index.js
module.exports = 1;
// @filename: local.d.ts
declare module "nonexistent" {
    export const k: number;
}
export {};
// @filename: a.ts
import "./local";
declare module "js" {
    export const j: number;
}
