// @isolatedModules: true
// @filename: bar.ts
export const BAR = 'bar';
export const NUM = 2;
// @filename: foo.ts
import { BAR, NUM } from './bar';
const LOCAL = 'LOCAL';
enum Foo {
  A = `${BAR}`,
  B = LOCAL,
  C = B,
  D = C + 'BAR',
  F = BAR,
  H = A,
  I = H + BAR,
  J = NUM,
}
