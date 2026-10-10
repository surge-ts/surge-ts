// A module-level `const` alias of a condition narrows by the condition it was
// written as and by its own truthiness, as a function body does.
declare const inBrowser: boolean;
declare const source: { mark(): void };
const perf = inBrowser && source;
if (perf && perf.mark) { perf.mark(); }
if (perf) { perf.mark(); }
declare const v: string | number;
const isString = typeof v === "string";
if (isString) { const s: string = v; } else { const n: number = v; }
declare const flag: boolean;
const isNumber = flag && typeof v === "number";
if (isNumber) { const n: number = v; }
