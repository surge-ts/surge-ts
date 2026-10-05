// @strict: true
let twelve = (f => f(12))(i => i);
let eleven = (o => o.a(11))({ a: function(n) { return n; } });
let ten = ((f: any) => f(12))(j => j);
const g = (f => 1);
g(k => k);
