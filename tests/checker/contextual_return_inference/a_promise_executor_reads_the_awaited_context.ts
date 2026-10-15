// @strict: true
// @lib: es2022
export async function returned(): Promise<void> {
  return new Promise((resolve) => { resolve(); });
}
export async function awaited(): Promise<void> {
  await new Promise((resolve) => { resolve(); });
}
export async function mismatched(): Promise<number> {
  return new Promise((resolve) => { resolve("x"); });
}
declare function takesVoid(fn: () => void): void;
takesVoid(async () => { return new Promise((resolve) => { resolve(); }); });
takesVoid(async () => new Promise((resolve) => { resolve(); }));
interface Overloaded {
  (title: string, fn?: (done: () => void) => void): void;
  (title: string, fn?: () => PromiseLike<any>): void;
}
declare const overloaded: Overloaded;
overloaded("a", async () => { return new Promise((resolve) => { resolve(); }); });
