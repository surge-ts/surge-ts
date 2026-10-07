// @target: esnext
async function* a(): AsyncGenerator<any, { x: "x" }, any> { const r = { x: "x" }; return r; }
async function* b(): AsyncGenerator<any, { x: "x" }, any> { const r = { x: "x" }; return r; }
function* c(): Generator<any, { x: "x" }, any> { const r = { x: "x" }; return r; }
function* d(): Generator<any, { x: "x" }, any> { const r = { x: "x" }; return r; }
async function* e(): AsyncGenerator<any, { x: "x" }, any> { const r = { x: "x" }; return Promise.resolve(r); }
