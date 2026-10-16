// @surge-compare: messages
declare let f1: (x?: number) => void;
const a1: number = f1;
interface I { m(x?: number): void; p: (y?: string | undefined) => void }
declare let i: I;
const a3: number = i.m;
const a4: number = i.p;
