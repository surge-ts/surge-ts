// A property declared after a same-named accessor merges into the accessor's
// symbol, so it must have the accessor's type (TS2717).
class AccessorFirst {
    get value() {
        return "x";
    }
    set value(v: string) {}
    value = 0;
}
class SameType {
    get label(): string {
        return "x";
    }
    label = "y";
}
