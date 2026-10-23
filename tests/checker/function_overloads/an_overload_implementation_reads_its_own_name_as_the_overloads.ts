// @strict: true
// Inside an overloaded function's implementation its own name is the
// overload group, which hides the implementation's signature: a recursive
// call resolves against the overloads.
function local(value: string): string;
function local(value: number): number;
function local(value: unknown): unknown {
  const recursed: string = local(1);
  return recursed;
}

function single(value: number): number {
  const recursed: string = single(value);
  return value;
}

export {};
