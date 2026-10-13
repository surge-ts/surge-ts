type Both<T> = T extends (...args: infer P) => (...args: any[]) => infer R ? [P, R] : 0;
declare const notCurried: Both<(x: number) => { type: string }>;
declare const curried: Both<(x: number) => (y: string) => boolean>;
const zero: 0 = notCurried;
const pair: [[x: number], boolean] = curried;
const wrong: string = notCurried;
