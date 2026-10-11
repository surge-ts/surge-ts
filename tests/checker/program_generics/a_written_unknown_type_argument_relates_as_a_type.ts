interface Val<T> { v: T; w: string }
declare const vu: Val<unknown>;
const s1: Val<string> = vu;
const s2: { v: string } = vu;
type Al<T> = { v: T };
declare const au: Al<unknown>;
const s3: Al<string> = au;
declare const vany: Val<any>;
const s4: Val<string> = vany;
const s5: Val<unknown> = null! as Val<string>;
function take(p: { v: string }) {}
take(vu);
