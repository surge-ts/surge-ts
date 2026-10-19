// An optional element read of a possibly undefined tuple intersection reads
// the tuple operand's element, with the `undefined` the chain adds.
type Tagged<T> = T & { tag: 1 };
declare function pair(): Tagged<[number, string]> | undefined;
const second: boolean = pair()?.[1];
const call: boolean = pair?.()?.[0];
