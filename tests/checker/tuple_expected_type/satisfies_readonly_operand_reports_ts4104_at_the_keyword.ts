const tuple = [10, "20"] as const;
const a = tuple satisfies number[];
declare function takes(...args: number[]): void;
takes(10, ...(tuple satisfies number[]));
const ro: readonly number[] = [1];
const b = ro /* note */ satisfies number[];
const c = 1 satisfies string;
