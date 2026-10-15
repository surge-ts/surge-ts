// @strict: true
declare function elem<T>(p: { prop: T; children: (t: T) => T }): T;
const widened = elem({ prop: { a: "x" }, children: (i) => ({ a: "z" }) });
const page = elem({ prop: 1, children: (last) => last + 1 });
const check: [] = [widened, page];
