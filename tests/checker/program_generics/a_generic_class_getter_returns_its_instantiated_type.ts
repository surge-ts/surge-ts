// An unannotated getter of a generic class whose body returns something typed
// by the class's own parameters is read again under each instantiation.
class Box<T> {
    _value!: T;
    _make!: () => T;
    get value() { return this._value; }
    get made() { return this._make(); }
    get count() { return 1; }
}
declare const box: Box<{ f1: number }>;
const value: string = box.value;
const made: string = box.made;
const indexed: string = null as any as Box<{ f1: number }>["made"];
const count: string = box.count;
