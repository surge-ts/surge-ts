// @allowJs: true
// @checkJs: true
// @filename: check.js
function container() {
    import "fs";
}
{
    import { x } from "./other";
}
// @filename: other.ts
export const x = 1;
function inTs() {
    import "fs";
}
