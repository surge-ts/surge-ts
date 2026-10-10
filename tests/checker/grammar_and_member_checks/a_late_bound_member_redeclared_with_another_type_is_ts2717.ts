// A computed member name whose key is a literal is late-bound to the name
// its value spells, bound after the static names (`lateBindMember`): a
// later property of that name with another type is TS2717.
const c0 = "1";
const c1 = 1;
interface T3 {
    [c0]: number;
    [c1]: string;
}
interface T5 {
    [c0]: number;
    "1": string;
}
