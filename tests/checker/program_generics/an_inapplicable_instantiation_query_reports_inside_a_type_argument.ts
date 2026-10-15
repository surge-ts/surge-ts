// @strict: true
const g = <N extends string, QR>(q: QR) => q;
type Direct = typeof g<number>;
type Wrapped = Array<typeof g<number>>;
declare const h: <N extends string, QR>(q: QR) => QR;
type Declared = Array<typeof h<number>>;
