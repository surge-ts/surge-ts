function func(): typeAlias3 { return null!; }
var varOfAliasedType3 = func();
type typeAlias3 = typeof varOfAliasedType3;

interface Input {
  a: number;
  b: number;
}

type R = ReturnType<typeof mul>;
function mul(input: Input): R {
  return input.a * input.b;
}

type R2 = ReturnType<typeof f>;
function f(): R2 { return 0; }

type Fine = ReturnType<typeof g>;
function g(): number { return 0; }
