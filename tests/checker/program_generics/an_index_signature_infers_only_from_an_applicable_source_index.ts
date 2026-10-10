// `inferFromIndexTypes`: a target's string index reads only the source's
// string index, its number index the number index or else the string one.
// With nothing to read, `T` is `unknown` and the argument is then checked.
interface NumberMap<T> {
    [index: number]: T;
}
interface StringMap<T> {
    [index: string]: T;
}
declare function numberMapToArray<T>(object: NumberMap<T>): T[];
declare function stringMapToArray<T>(object: StringMap<T>): T[];
declare var numberMap: NumberMap<Function>;
declare var stringMap: StringMap<Function>;
var v1: Function[];
var v1 = numberMapToArray(numberMap);
var v1 = numberMapToArray(stringMap);
var v1 = stringMapToArray(numberMap);
var v1 = stringMapToArray(stringMap);
