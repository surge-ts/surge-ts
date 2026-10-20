// tsc's `assignParameterType`: a binding pattern contextually typed
// `unknown` takes its pattern's implied type, whose elements are `any`.
declare function query(fn: (opts: unknown) => unknown): unknown;
query(({ input }) => input.length);
query(([first]) => first.size);
