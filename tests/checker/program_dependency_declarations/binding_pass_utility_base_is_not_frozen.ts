// @filename: options.ts
export interface RequestOptions { context?: { trace: string }; signal?: AbortSignal }

// @filename: react.ts
import type { RequestOptions } from './options';
export interface ReactRequestOptions extends Omit<RequestOptions, 'signal'> { ssr?: boolean }
export interface BaseOptions { trpc?: ReactRequestOptions }

// @filename: context.ts
import type { BaseOptions } from './react';
declare function makeContext<T>(): { value: T };
export const ctx = makeContext<BaseOptions | null>();

// @filename: call.ts
import { ctx } from './context';
import type { RequestOptions } from './options';
export function call(send: (options?: RequestOptions) => void) {
  send(ctx.value?.trpc);
}
