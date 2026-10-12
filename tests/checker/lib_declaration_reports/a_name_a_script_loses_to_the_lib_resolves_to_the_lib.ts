// @lib: es2023,dom
// A script declaration that cannot merge with the library's global of its
// name stays out of the global scope (`mergeSymbol` keeps the library's
// symbol), so every read of the name, the script's own included, is the
// library's.
var name: string = "my name";
var person: { name: string } = { name };
let origin = 5;
origin.toFixed();
class Symbol { bound = false; }
class InferenceSymbol extends Symbol { }
declare var inference: InferenceSymbol;
inference.bound;
class Derived extends BigInt { }
