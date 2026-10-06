declare var strOrNumber: string | number;
declare var str: string;
var objStrOrNum5: { prop: string; anotherP: string; } | { prop: number } = { prop: strOrNumber };
var objStrOrNum6: { prop: string; anotherP: string; } | { prop: number } = {
    prop: strOrNumber,
    anotherP: str
};
var o3: { prop: string } | { prop: number } = { prop: strOrNumber };
var o4: { a: string; b: number } | { a: number; c: string } = { a: "x", b: "y" };
