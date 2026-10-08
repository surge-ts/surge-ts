// @lib: esnext
// @target: esnext
interface CustomPromisifyLegacy<TCustom extends Function> extends Function { __promisify__: TCustom; }
type CustomPromisify<TCustom extends Function> = CustomPromisifyLegacy<TCustom>;
declare function promisify<TCustom extends Function>(fn: CustomPromisify<TCustom>): TCustom;
declare function promisify(fn: (callback: (err: any) => void) => void): () => Promise<void>;
declare function promisify<T1>(fn: (arg1: T1, callback: (err: any) => void) => void): (arg1: T1) => Promise<void>;
declare function exec(command: string, callback?: (error: Error | null, stdout: string, stderr: string) => void): void;
declare namespace exec {
  function __promisify__(command: string): Promise<{ stdout: string; stderr: string }>;
}
export const execa = promisify(exec);
export async function status() {
  const { stdout } = await execa("git status");
  return stdout;
}
interface Tagged extends Function { tag: number }
declare const tagged: { (a: string): void; tag: number };
declare function pick(x: Tagged): 1;
declare function pick(x: (a: string) => void): 2;
export const picked: 1 = pick(tagged);
