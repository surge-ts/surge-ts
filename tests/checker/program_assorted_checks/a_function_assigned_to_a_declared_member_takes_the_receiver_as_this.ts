declare const obj: { k: () => void; s: string };
obj.k = function () { const y: number = this.s; };
function inner() {
    obj.k = function () { const y2: number = this.s; };
}
declare const loose: { k: any; s: string };
loose.k = function () { const y3: number = this.s; };
interface Bag { [k: string]: (() => void) | number; }
declare const bag: Bag;
bag["f"] = function () { const q: string = this.zz; };
declare const withThis: { f: (this: { w: boolean }) => void };
withThis.f = function () { const t: string = this.w; };
declare const unionThis: { f: ((this: { name: string }) => void) | null };
unionThis.f = function () { const u: string = this.name; };
obj.k = () => { const arrow: number = 1; };
export {};
