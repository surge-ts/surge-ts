// @allowJs: true
// @checkJs: true
// @target: esnext
// @useDefineForClassFields: false
// @filename: a.js
class C {
    /**
     * @type {unique symbol}
     */
    static readwriteStaticType;
    /**
     * @type {unique symbol}
     * @readonly
     */
    static readonlyType;
    /**
     * @type {unique symbol}
     */
    readwriteType;
}
