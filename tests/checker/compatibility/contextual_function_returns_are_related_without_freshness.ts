// @strict: true
// @lib: es2023
type Sync = (opts: { a: number }) => { ok: boolean };
type Async = (opts: { a: number }) => Promise<{ ok: boolean }>;
const expressionBody: Sync = (opts) => ({ ok: true, extra: opts.a });
const blockBody: Sync = (opts) => { return { ok: true, extra: 1 }; };
const functionBody: Sync = function (opts) { return { ok: true, extra: 1 }; };
const asyncArrow: Async = async (opts) => ({ ok: true, extra: 1 });
const asyncFunction: Async = async function (opts) { return { ok: true, extra: 1 }; };
declare function takeAll(m: { middlewares: Async[] }): void;
takeAll({ middlewares: [async function named(opts) { return { ok: true, extra: opts.a } as const; }] });
const nestedCallback: Sync = (opts) => ({ ok: true, inner: { deep: 1 } });
const stillChecked: Sync = (opts) => ({ ok: 1 });
const stillCheckedBlock: Sync = (opts) => { return { ok: 1, extra: 1 }; };
declare function f(x: { ok: boolean }): { ok: boolean };
const argumentInReturn: Sync = (opts) => { return f({ ok: true, extra: 1 }); };
const argumentInBody: Sync = (opts) => f({ ok: true, extra: 1 });
const annotatedInner: () => { cb: () => { a: number } } = () => ({ cb: (): { a: number } => ({ a: 1, extra: 2 }) });
const declarationInBody: Sync = (opts) => { const v: { ok: boolean } = { ok: true, extra: 2 }; return v; };
function annotated(): { ok: boolean } { return { ok: true, extra: 1 }; }
