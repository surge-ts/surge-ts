// @surge-compare: messages
const f: (() => string) | (() => number) = 1;
declare let g: { m(...items: string[]): void };
const s: string = g;
