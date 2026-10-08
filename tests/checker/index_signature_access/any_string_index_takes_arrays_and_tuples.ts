declare function config<TConfig extends Record<string, any> | undefined>(a: unknown): TConfig;

export const tuple = config<readonly [string, ...string[]] | [string, ...string[]]>(1);
export const array = config<string[]>(1);
export const readonlyArray = config<readonly string[]>(1);

export const r1: Record<string, any> = ['a'] as readonly [string, ...string[]];
export const r2: Record<string, any> = ['a'] as [string, ...string[]];
export const r3: { [key: string]: any } = [1, 2];
export const r4: Record<string, unknown> = ['a'] as readonly string[];
export const r5: Record<string, any> = 'text';
