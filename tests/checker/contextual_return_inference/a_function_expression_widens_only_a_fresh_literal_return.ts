// @surge-compare: messages
// A function expression's literal return widens only when it is fresh
// (`getWidenedLiteralType`): a parameter, an annotated binding, a declared
// property or an assertion reads a regular literal, a written literal or an
// unannotated `const` a fresh one. A block body widens the union of its
// returns, so two literals, or one and `undefined`, stay a union.
declare const c: boolean;
declare function lit(): 5;
declare const o: { kind: "a" };
declare const t: readonly [5];
enum E { A, B }
const K = 5;
const KA: 5 = 5;

const f1 = (n: 5) => n;                              const r1: 5 = f1(5);
const f2 = (n: 5) => { return n; };                  const r2: 5 = f2(5);
const f3 = function (n: 5) { return n; };            const r3: 5 = f3(5);
const f4 = ((n: 5) => n)(5);                         const r4: 5 = f4;
let f5 = async (n: 5) => n;                          const r5: Promise<5> = f5(5);
const f6 = () => KA;                                 const r6: 5 = f6();
const f7 = () => 5 as 5;                             const r7: 5 = f7();
const f8 = () => lit();                              const r8: 5 = f8();
const f9 = () => o.kind;                             const r9: "a" = f9();
const f10 = () => t[0];                              const r10: 5 = f10();
const f11 = (n: 5 | undefined) => n!;                const r11: 5 = f11(5);
const f12 = (n: 5) => (c ? n : K);                   const r12: 5 = f12(5);
const f13 = () => { if (c) return 1; return 2; };    const r13: 1 | 2 = f13();
const f14 = () => { if (c) return 5; };              const r14: 5 | undefined = f14();
const f15 = (n: 1) => { if (c) return n; return 1; }; const r15: 1 = f15(1);

const w1 = () => K;                                  const s1: 5 = w1();
const w2 = () => { const k = 5; return k; };         const s2: 5 = w2();
const w3 = () => E.A;                                const s3: E.A = w3();
const w4 = (n: number) => (n === 5 ? n : 5);         const s4: 5 = w4(5);
const w5 = () => { if (c) return 1; return 1; };     const s5: 1 = w5();
function w6(n: 1) { if (c) return n; return 1; }     const s6: 1 = w6(1);
export {};
