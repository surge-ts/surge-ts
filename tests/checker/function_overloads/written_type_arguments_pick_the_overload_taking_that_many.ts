interface fn2 { (s: string, n: number): number; <T>(n: number, t: T): T; }
declare var fn2: fn2;
fn2<Date>('', 0);
interface ctor2 { new (s: string, n: number): number; new <T>(n: number, t: T): T; }
declare var ctor2: ctor2;
new ctor2<Date>('', 0);
