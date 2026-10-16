export const texts: string[] = [];
// @ts-expect-error
texts.push(100);
// @ts-expect-error
texts.push("100");
// @ts-expect-error
const a: string = 1;
// @ts-expect-error
const b: string = "ok";
// @ts-ignore
const c: string = "ok";
