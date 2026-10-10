// A default tuple shorter than its array pattern is padded with the pattern's
// remaining elements (`padTupleToMatchBindingPattern`).
function g4([x, y = 0] = [0]) {}
g4();
g4([1, 1]);
function g5([x, y] = [0]) {}
g5([1, 1]);
