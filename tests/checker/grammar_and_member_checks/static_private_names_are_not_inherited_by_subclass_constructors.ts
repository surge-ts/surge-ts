// @target: esnext
class Parent {
    #foo = 3;
    static #bar = 5;
    accessChildProps() {
        new Child().#foo;
        Child.#bar;
        Parent.#bar;
    }
}
class Child extends Parent {
    #baz = "baz";
}
