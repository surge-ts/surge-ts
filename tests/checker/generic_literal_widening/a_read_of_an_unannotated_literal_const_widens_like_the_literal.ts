declare function seq<T>(a: T): (b: T) => T;
declare function id<T>(a: T): T;
const text1 = "hello";
const text2 = "world";
const annotated: "hello" = "hello";
var text3: string = seq(text1)(text2);
const kept: "hello" = id(text1);
const strict = seq(annotated);
strict(text2);
