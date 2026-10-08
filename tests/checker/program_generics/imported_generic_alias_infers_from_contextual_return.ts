// @filename: lib.ts
export type Creator<T, M extends unknown[] = []> = ((
  set: 'outer' extends M[number] ? (value: T, tag: string) => void : (value: T) => void,
) => T) & { $$marks?: M };

type Wrap = <T, M extends unknown[] = []>(init: Creator<T, [...M, 'wrap']>) => Creator<T, M>;
export declare const wrap: Wrap;

type Outer = <T, M extends unknown[] = []>(init: Creator<T, [...M, 'outer']>) => Creator<T, M>;
export declare const outer: Outer;

// @filename: main.ts
import { outer, wrap } from './lib';

export const tagged = outer<number>(
  wrap((set) => {
    set(1, 'tag');
    return 1;
  }),
);
