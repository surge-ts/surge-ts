// @filename: lib.ts
export const resultOf = <T, TArgs extends any[]>(value: T | ((...args: TArgs) => T), ...args: TArgs): T => {
  return typeof value === 'function' ? (value as (...args: TArgs) => T)(...args) : value;
};
export type CallbackOrValue<T> = T | (() => T | Promise<T>);
export interface Options { url: CallbackOrValue<string> }
// @filename: main.ts
import { resultOf, type Options } from "./lib";
export async function f(opts: Options) {
  const url = await resultOf(opts.url);
  const hasQuery: boolean = url.includes("?");
  const wrong: 1 = url;
}
