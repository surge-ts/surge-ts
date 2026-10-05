class Foo { protected x!: string; }
class Baz { protected y!: string; }
interface I5 extends Foo, Baz { z: string; }
declare var i: I5;
var r2 = i.x;
var r3 = i.y;
interface I6 extends Baz { }
declare var j: I6;
var r4 = j.y;
