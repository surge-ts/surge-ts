// @strict: false
// @noImplicitAny: true
var o = { value: null, b: undefined, c: 1 };
let p = { q: { r: null }, s: [null] };
const arr = [null, undefined];
var [a, b] = [undefined, null];
var { c, d } = { c: null, d: 1 };
var notReported = null;
