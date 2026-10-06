class Base { private m: number = 0; }
class Derived extends Base { private q: number = 0; }
const f1 = function () { if (Math.random()) return new Base(); return new Derived(); };
const r1: 7 = f1();
const f3 = () => { if (Math.random()) return new Derived(); return new Base(); };
const r3: 7 = f3();
var b: Base;
var b = function () { if (Math.random()) return new Base(); return new Derived(); }();
