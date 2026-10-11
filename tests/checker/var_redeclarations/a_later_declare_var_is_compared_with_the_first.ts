// @strict: false
// A `declare var` in a source file is one more declaration of the `var`: its
// type is compared with the first declaration's, whichever order they come in
// (TS2403 at the later one).
declare function f<T>(a: T, b: T): T;
var inferred = f('', '');
declare var inferred: {};

declare var ambientFirst: number;
var ambientFirst = 1;

declare var twice: number;
declare var twice: string;

var same = "";
declare var same: string;
