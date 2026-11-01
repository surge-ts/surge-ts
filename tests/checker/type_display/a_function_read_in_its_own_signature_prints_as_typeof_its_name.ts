// @surge-compare: messages
// @strict: true
// @target: es2015
// @jsx: preserve
// @filename: example.tsx
import * as React from "react";
declare global { namespace JSX { type ElementType = string | ((props: any) => any); interface IntrinsicElements {} } }
type Box<T> = { v: T };
function F(props: {}, ref: React.ForwardedRef<typeof F>) { return null; }
<F />;
const f: (p: any) => any = F;
function B(props: {}, ref: Box<typeof B>) { return null; }
const b: number = B;
declare function P(ref: typeof P): null;
const p: number = P;
function R(): typeof R { return R; }
const r: number = R;
function I(cb = I) { return 1; }
const i: number = I;
export function E(props: {}, ref: Box<typeof E>) { return null; }
const e: number = E;
declare function O(n: number, r: typeof O): void;
declare function O(s: string): void;
const o: number = O;
namespace N {
    function L(ref: Box<typeof L>) { return null; }
    const l: number = L;
}
declare function Q(x: typeof later): void;
declare function later(n: number): string;
const q: number = Q;
function g(cb = later2) { return 1; }
function later2(s: string) { return s; }
const h: number = g;
