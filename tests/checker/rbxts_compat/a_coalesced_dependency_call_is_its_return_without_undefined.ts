// @types: lua
// `f() ?? fallback` over a function a dependency declares, whose annotation
// is read lazily (`tonumber(…): number | undefined` from @rbxts/types), is the
// return without `undefined`, as over any other `number | undefined`.
// @filename: node_modules/@types/lua/package.json
{ "name": "@types/lua", "version": "1.0.0", "types": "index.d.ts" }
// @filename: node_modules/@types/lua/index.d.ts
declare function tonumber(arg: unknown, base?: number): number | undefined;
declare namespace math {
	function log10(n: number): number;
}
// @filename: index.ts
declare const text: string | undefined;
const fromCall = math.log10(tonumber(text) ?? 1);
const read = tonumber(text);
const fromLocal: number = read ?? 1;
const mismatched: string = tonumber(text) ?? 1;
export {};
