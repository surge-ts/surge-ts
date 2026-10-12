var generic1 = <T>(n: T) => [n];
var generic1: { <T>(n: T): T[] };
var generic2 = <T>(n: T) => { return [n]; };
var generic2: <U>(n: U) => U[];
var different = <T>(n: T) => n;
var different: { <T>(n: T): T[] };
