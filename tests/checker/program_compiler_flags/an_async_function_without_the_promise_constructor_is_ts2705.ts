// @lib: es5
// @filename: b.ts
export const x = 1;
// @filename: a.ts
const f = async () => {};
async function g() {}
const i = import("./b");
export {};
