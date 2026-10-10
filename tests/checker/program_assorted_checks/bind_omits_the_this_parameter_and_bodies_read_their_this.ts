// @strict: true
// @lib: es2023
function count(this: { n: number }) { return this.n; }
const bound = count.bind({ n: 1 });
const boundResult: string = bound();
function scaled(this: { n: number }, x: string) { return x.length + this.n; }
const scaledBound = scaled.bind({ n: 1 });
const scaledResult: string = scaledBound("a");
const partial = scaled.bind({ n: 1 }, "a");
const partialResult: string = partial();
const called: string = count.call({ n: 1 });
const applied: string = scaled.apply({ n: 1 }, ["a"]);
