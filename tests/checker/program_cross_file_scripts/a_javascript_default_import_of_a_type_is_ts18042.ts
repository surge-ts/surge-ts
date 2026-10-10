// @allowJs: true
// @checkJs: true
// @filename: types.ts
// A JavaScript file imports values only (`checkAliasSymbol`): a default
// export with no value meaning is TS18042.
export default interface Shape {
    size: number;
}
// @filename: use.js
import Shape from "./types";
