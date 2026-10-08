declare function id<T>(x: T): T;
declare function lit(): "a";
declare const k: "b";
function f1() { return id("x"); }
function f2() { return id<"y">(null!); }
function f3() { return lit(); }
function f4() { return k; }
function f5() { return "z"; }
function f6() { return Math.random() ? id("x") : id("w"); }
const p1: 1 = f1(); const p2: 1 = f2(); const p3: 1 = f3(); const p4: 1 = f4(); const p5: 1 = f5(); const p6: 1 = f6();
