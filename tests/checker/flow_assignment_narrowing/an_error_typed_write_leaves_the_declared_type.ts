// Writing a value of tsc's error type (read off a name that does not resolve)
// leaves the binding its declared type, as an `any` write does.
class Animal {
    a = 1;
}
declare const missing: Missing;
function straight() {
    var c: Animal = null as any;
    c = missing.x;
    c.bark();
}
function branched(b: boolean) {
    var c: Animal = null as any;
    if (b) {
        c = missing.x;
    }
    c.bark();
}
