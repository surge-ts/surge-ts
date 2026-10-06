function fib(n: number) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function g() { return g(); }
function h(n: number) { if (n) return 1; return h(n - 1); }
function sum(xs: number[]): number { return xs.length ? xs[0] + sum(xs.slice(1)) : 0; }
function nested(n: number) { if (n > 0) { return [nested(n - 1)]; } return []; }
