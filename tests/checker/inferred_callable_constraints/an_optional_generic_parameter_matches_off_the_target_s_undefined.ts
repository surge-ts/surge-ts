// @strict: true
declare let s4: <T>(x?: T) => T;
declare let t4: (x?: string) => string;
t4 = s4;
declare let s2: <T>(x: T) => T;
declare let t2: (x?: string) => string;
t2 = s2;
interface Base2 { a4: <T>(x: T, y?: T) => T; }
interface I12 extends Base2 { a4: <T>(x?: T, y?: T) => T; }
interface I14 extends Base2 { a4: <T>(x: T, y: T) => T; }
