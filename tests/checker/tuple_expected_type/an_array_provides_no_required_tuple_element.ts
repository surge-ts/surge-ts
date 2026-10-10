// @strict: true
// @lib: es2023
declare function nonEmpty(x: [number, ...number[]]): void;
declare const xs: number[];
nonEmpty([1, 2]);
nonEmpty([]);
nonEmpty(xs);
nonEmpty([...xs]);
nonEmpty([1, ...xs]);

declare function lastIsString(x: [...number[], string]): void;
lastIsString(xs);
declare function optionalHead(x: [number?, ...number[]]): void;
optionalHead(xs);
declare function onlyRest(x: [...number[]]): void;
onlyRest(xs);

const declared: [string, ...number[]] = ["a", 1, 2];
const literalOnly: [string, ...number[]] = ["a"];
const fromArray: [number, ...number[]] = xs;
declare function produce(f: () => [number, ...number[]]): void;
produce(() => [1]);
produce(() => xs);
