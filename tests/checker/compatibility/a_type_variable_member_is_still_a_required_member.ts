type Record2<K extends keyof any, T> = { [P in K]: T };

function m1<T>(x: { a: T }, y: { b: T }) { x = y; }
function m2<T>(x: { a: T }, y: { [k: string]: T }) { x = y; }
function m3<T>(x: Record2<"a", T>, y: Record<string, T>) { x = y; }
function m4<T>(x: Record<"a", T>, y: Record2<string, T>) { x = y; }
function m5<T>(y: { [k: string]: T }) { const x: { a: T } = y; return x; }
function ok<T>(x: { a?: T }, y: { [k: string]: T }) { x = y; }

export { m1, m2, m3, m4, m5, ok };
