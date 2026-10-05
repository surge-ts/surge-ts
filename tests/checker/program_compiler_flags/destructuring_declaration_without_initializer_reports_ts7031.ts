// @noImplicitAny: true
var [a], {b}, c, d;
var [a1 = undefined], {b1 = null}, c1 = undefined, d1 = null;
var [a2]: [any], {b2}: { b2: any }, c2: any, d2: any;
let [[n1], { m: n2 }] = [[1], { m: 2 }];
