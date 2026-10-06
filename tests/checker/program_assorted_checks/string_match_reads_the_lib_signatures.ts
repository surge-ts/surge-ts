declare const s: string;
const c = s.match("a");
const n1: number = c;
const d = s.match(/a/g);
d.length;
const n2: number = d![0];
