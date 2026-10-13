// A construct signature pairs with a class's: its return type is inferred
// from the instance, so a static member infers nothing and `T` is `unknown`.
declare function make<T>(ctor: new () => { prop: T }): T;
make(class { static prop = "hello" }).length;
const fromInstance: number = make(class { prop = "hello" });
interface Ctor<T> { new (): { value: T } }
declare function build<T>(ctor: Ctor<T>): T;
build(class { static value = 1 }).toFixed();
const built = build(class { value = 1 });
const asString: string = built;
