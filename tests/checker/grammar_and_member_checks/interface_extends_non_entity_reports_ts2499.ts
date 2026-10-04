namespace Foo {
    export class Bar {}
}
interface C1 extends Foo?.Bar {}
class A { }
interface Class extends (typeof A) { }
