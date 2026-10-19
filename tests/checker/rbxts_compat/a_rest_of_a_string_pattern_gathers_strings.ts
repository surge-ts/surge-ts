// A string is iterated by code point: the rest of an array pattern over it
// gathers `string`s, as its other elements read one each.
export {};
const [h, e, ...llo] = "hello";
const joined: string = llo.join("");
const wrong: number = llo;
const [[first, ...inner], ...outer] = "xab";
const outerCount: number = outer.length;
const notArray: string = outer;
