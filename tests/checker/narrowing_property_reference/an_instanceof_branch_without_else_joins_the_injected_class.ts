// @strict: true
interface OnChanges { onChanges(changes: Record<string, unknown>): void }
interface Validator { validate(): null | Record<string, unknown>; }
class C { validate() { return {} } }
function locals(p: Validator & Partial<OnChanges>) {
    let v: Validator & Partial<OnChanges> = null as any;
    if (v instanceof C) { v; }
    if (v.onChanges) { v.onChanges({}); }
    if (p instanceof C) { p; }
    if (p.onChanges) { p.onChanges({}); }
}
class Animal { name = "" }
class Dog extends Animal { bark() {} }
function subtype(a0: Animal) {
    let a: Animal = a0;
    if (a instanceof Dog) { a.bark(); }
    a.name;
}
