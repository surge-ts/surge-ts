function p<T extends { a: string } | undefined>(obj: T) {
    obj.a;
    obj["a"];
    if (obj) {
        obj.a;
    }
}
function q<T extends { a: string }>(obj: T) {
    obj.a;
}
