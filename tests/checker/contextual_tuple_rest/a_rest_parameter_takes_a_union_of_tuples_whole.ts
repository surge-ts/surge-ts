type ArgsUnion = [number, string] | [number, Error];
type TupleUnionFunc = (...params: ArgsUnion) => number;
const first: TupleUnionFunc = (...params) => {
  const [num, strOrErr] = params;
  return num;
};
const indexed: TupleUnionFunc = (...params) => params[0];
const whole: TupleUnionFunc = (...params) => {
  const s: string = params;
  return 1;
};
