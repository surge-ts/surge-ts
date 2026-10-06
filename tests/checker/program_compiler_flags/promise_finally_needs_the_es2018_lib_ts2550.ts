// @lib: es2015
const a = new Promise(() => {}).finally();
declare const q: Promise<number>;
q.finally();
q.then(x => x).finally(() => {});
async function f() { return 1; }
f().finally();
