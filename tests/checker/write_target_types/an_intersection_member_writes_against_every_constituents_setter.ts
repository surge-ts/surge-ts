// A member of an intersection is written against the intersection of what
// each constituent accepts: a setter's parameter type where it declares one.
class Field {
    value: number = 42;
}
class Narrow {
    get value(): string {
        return "";
    }
    set value(next: string | 42) {}
}
class Wide {
    get value(): string {
        return "";
    }
    set value(next: string | number | boolean) {}
}
declare const fieldAndNarrow: Field & Narrow;
fieldAndNarrow.value = "text";
fieldAndNarrow.value = 42;
declare const narrowAndWide: Narrow & Wide;
narrowAndWide.value = true;
narrowAndWide.value = "text";
declare const all: Field & Narrow & Wide;
all.value = "text";
all["value"] = "text";
all.value = 42;
// Every constituent decides: one writable declaration keeps it writable.
class ReadOnly {
    readonly value: number = 1;
}
class Writable {
    value: number = 1;
}
declare const readOnlyFirst: ReadOnly & Writable;
readOnlyFirst.value = 2;
declare const writableFirst: Writable & ReadOnly;
writableFirst.value = 2;
export {};
