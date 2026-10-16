// @surge-compare: messages
namespace M {
    export class E<T> { e!: T; }
    export class Foo { private x = 1; }
}
var a: M.E;
class Bar extends M.Foo { y = this.x; }
