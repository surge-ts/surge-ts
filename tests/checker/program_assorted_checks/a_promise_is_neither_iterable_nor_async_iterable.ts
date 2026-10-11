// @target: es2018
declare const p: Promise<number[]>;
declare const pl: PromiseLike<number[]>;
async function* ag() { yield* p; }
async function* ag2() { yield* pl; }
function* sg() { yield* p; }
async function f() {
    for (const x of p) {}
    for await (const y of p) {}
    const arr = [...p];
    const [first] = p;
    Math.max(...p);
    for (const z of await p) {}
}
declare const maybe: Promise<number[]> | number[];
for (const z of maybe) {}
export {};
