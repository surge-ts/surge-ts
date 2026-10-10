// @strict: true
// `Pick<T, K>` is `{ [P in K]: T[P] }`: a `string` key picks `T`'s string
// index signature, and a literal key `T` does not declare reads that index.
type Dict = { [key: string]: { a: string } };
declare const all: Pick<Dict, keyof Dict>;
all["x"].b;
declare const some: Pick<Dict, "x" | "y">;
some.x.b;
some.x.a;
type Constr<Source, Target> = { [K in keyof Source]: string } & Pick<Target, Exclude<keyof Target, keyof Source>>;
declare const q: Constr<{}, Dict>;
q["asd"].a.substr(1);
q["asd"].b;
const d: { [key: string]: { a: string; b: string } } = q;
export {};
