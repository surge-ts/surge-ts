// @lib: es2023,dom
// A script's declarations merge with the default library's globals, and one
// that cannot merge is a duplicate at every declaration of both, the lib's
// included (`reportMergeSymbolError`).
let name = "x";
type PropertyKey = string;
interface ElementTagNameMap {
    x: number;
}
class Symbol {}
