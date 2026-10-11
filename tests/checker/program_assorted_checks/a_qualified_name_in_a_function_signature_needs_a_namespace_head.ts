var Emu5 = {};
class Outer5 {}
function f1(x: Emu5.D) {}
function f2(y: Outer5.Inner) {}
function f3(): Emu5.D { return null!; }
function f4(z: Missing5.D) {}
namespace NS { export interface T {} }
function f5(w: NS.T) {}
function f6(v: NS.Missing) {}
function f7<T>(u: T.x) {}
