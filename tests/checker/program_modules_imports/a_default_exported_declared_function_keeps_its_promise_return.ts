// @strict: true
// @module: esnext
// @moduleResolution: bundler
// @filename: pkg/glue.d.ts
export interface InitOutput {
  readonly memory: number;
}
export default function init(path?: string): Promise<InitOutput>;
// @filename: pkg/glue.js
export default async function init() {}
// @filename: index.ts
import init from "./pkg/glue.js";
let loading: Promise<unknown> | undefined;
export function load(): Promise<unknown> {
  if (!loading) {
    loading = init("x").catch((error) => {
      loading = undefined;
      throw error;
    });
  }
  return loading;
}
export const wrong: number = init();
