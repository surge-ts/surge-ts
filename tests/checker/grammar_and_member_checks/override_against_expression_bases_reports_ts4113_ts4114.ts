// @noImplicitOverride: true
class E extends (class {
    foo() { }
    bar() { }
}) {
    override foo() { }
    bar() { }

    baz() {}

    override bazz() {}
}

type Foo = abstract new (...args: any) => any;
declare function CreateMixin<C extends Foo, T extends Foo>(Context: C, Base: T): T & {
    new (...args: any[]): { context: InstanceType<C> }
};
class Context {}

class A {
    doSomething() {}
}

class B extends CreateMixin(Context, A) {
    override foo() {}
}

class C extends CreateMixin(Context, A) {
    override doSomethang() {}
}

declare function plain(): new () => { known(): void };
class D extends plain() {
    override known() {}
    override unknown() {}
}
