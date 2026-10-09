// @strict: true
declare const u1: { [a: string]: number } | boolean;
const r1 = u1[10];
declare const u2: { 0: string } | { 0: number };
const r2: boolean = u2[0];
declare const u3: { 0: string } | { 1: number };
const r3 = u3[0];
declare const u4: { [n: number]: number } | { [s: string]: string };
const r4: boolean = u4[0];
declare const u5: string[] | { 0: number };
const r5: boolean = u5[0];
declare const u6: { [s: string]: number } | { [s: string]: string };
const r6: boolean = u6[0];
declare let w: { s: number; "0": number } | { [n: number]: number; [s: string]: string | number };
w[0] = 1;
w[1] = 0;
