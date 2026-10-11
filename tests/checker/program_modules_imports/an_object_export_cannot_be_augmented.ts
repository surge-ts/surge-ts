// @allowJs: true
// @checkJs: true
// A module whose `module.exports` is an object has no namespace to augment
// (TS2671), and the augmentation merges nothing: `a` keeps the object's type.
// @filename: test.js
module.exports = {
  a: "ok"
};
// @filename: index.ts
import { a } from "./test";
declare module "./test" {
  export const a: number;
}
a.toFixed();
