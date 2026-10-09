// @strict: true
// @filename: api/trace.ts
export declare class TraceAPI {
  getTracer(name: string): number;
}
// @filename: trace-api.ts
import { TraceAPI } from "./api/trace";
export declare const trace: TraceAPI;
export const num = 1;
export declare const str: string;
export function fn(): number { return 1; }
export declare const cls: TraceAPI;
export const fallback = { kind: "fallback" as const };
// @filename: index.ts
import { trace, num, str, fn, cls, fallback } from "./trace-api";
export { trace, num, str, fn };
export { cls as renamed, fallback as default };
// @filename: app.ts
import picked, { trace, num, str, fn, renamed } from "./index";
let a: string = trace;
let b: string = num;
let c: number = str;
let d: string = fn();
let e: string = renamed;
let f: "other" = picked.kind;
let g: string = trace.getTracer("x");
