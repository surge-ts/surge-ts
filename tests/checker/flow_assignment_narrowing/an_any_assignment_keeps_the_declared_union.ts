// tsc reduces a declared union by what was assigned, and `any` fits every
// member: the write leaves the whole declaration, `undefined` included, even
// after the binding was narrowed to one member.
declare function anyValue(): any;
declare const anyConstant: any;
function afterCall() {
    let r: number | undefined;
    r = anyValue();
    const y: string = r;
}
function afterNarrowing() {
    let r: number | undefined = 1;
    r = anyConstant;
    const y: string = r;
}
function afterParse() {
    let r: number | undefined;
    r = JSON.parse("1");
    r.toFixed();
}
let moduleLevel: number | undefined;
moduleLevel = anyConstant;
const fromModule: string = moduleLevel;
