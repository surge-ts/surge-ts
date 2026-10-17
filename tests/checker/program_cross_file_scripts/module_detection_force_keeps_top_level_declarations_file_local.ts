// @moduleDetection: force
// @filename: a.ts
const object = 1;
function f() {}
class C {}
interface I { a: number }
var v = 1;
const fromDeclaration: number = ambient;
// @filename: b.ts
const object = "2";
function f() {}
class C {}
interface I { b: string }
var v = "1";
const missing = object + fromDeclaration;
// @filename: globals.d.ts
declare const ambient: number;
// @filename: duplicate.d.ts
declare const ambient: number;
