// @strict: true
// A function expression's written `unknown` return is its return wherever
// it is read, an array literal's element included.
const fromArray = [function (): unknown { return "a"; }];
export const listed: (() => string)[] = fromArray;
const arrows = [(): unknown => "a"];
export const listedArrows: (() => string)[] = arrows;
