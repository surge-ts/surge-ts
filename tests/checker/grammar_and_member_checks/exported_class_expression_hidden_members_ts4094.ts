// @declaration: true
// @target: es2022
export var noPrivates = class {
    static getTags() { }
    private static ps = -1
    private p = 12
    protected q = 1
    #r = 2
}
export const withBase = class extends Object {
    private hidden = 1
}
const notExported = class {
    private p = 1
}
