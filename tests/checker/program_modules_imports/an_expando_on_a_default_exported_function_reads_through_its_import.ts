// @allowJs: true
// @checkJs: true
// @filename: a.js
export default function MyClass() {}
MyClass.bar = class C {};
MyClass.bar;
// @filename: b.js
import MC from "./a";
MC.bar;
MC.missing;
