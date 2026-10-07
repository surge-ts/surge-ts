// @target: esnext
async function a(): Promise<{ x: "x" }> { const r = { x: "x" }; return r; }
async function b(): Promise<{ x: "x" }> { const r = { x: "x" }; return r; }
function* c(): Generator<{ y: "y" }, void, any> { const r = { y: "y" }; yield r; }
function* d(): Generator<{ y: "y" }, void, any> { const r = { y: "y" }; yield r; }
function* e(): Generator<number, void, string> { const s: number = yield 1; }
function* f(): Generator<number, void, string> { const s: number = yield 1; }
