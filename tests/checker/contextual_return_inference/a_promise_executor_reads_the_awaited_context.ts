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
