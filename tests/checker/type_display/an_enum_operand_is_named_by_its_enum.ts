// @surge-compare: messages
enum E { A, B }
interface I { x: number }
declare var e: E;
declare var i: I;
i + e;
e + i;
