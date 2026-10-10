// @lib: es2023,dom
// @skipLibCheck: true
// tsc reports a lib file's diagnostics only when it checks the file.
let name = "x";
type PropertyKey = string;
interface Object {
    [key: string]: Object;
}
interface Function {
    [n: number]: string;
}
function print(x: number) {}
