// @surge-compare: messages
class Base { a = 1 }
class Derived extends Base { b = 1 }
declare let f: (x: Array<Base>) => Array<Derived>;
const n: number = f;
null as Array<Derived>;
