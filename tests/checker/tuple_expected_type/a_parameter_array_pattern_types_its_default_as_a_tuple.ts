// An unannotated array-pattern parameter types its default in a tuple
// context: the parameter is the tuple the default writes.
function f([a, b] = [1, "x"]) {}
f([2, 3]);
function g([[a], b] = [[undefined], undefined]) {}
g([["s"], 1]);
const h = ([a, b] = [true, 0]) => a;
h([false, "no"]);
