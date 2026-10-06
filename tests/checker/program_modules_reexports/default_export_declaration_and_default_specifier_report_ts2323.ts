// @module: commonjs
// @filename: c.ts
export const y = 1;
export default function named() {}
export { y as default };
// @filename: b.ts
export default class X {}
export { y as default } from './c';
// @filename: a.ts
export default function () { }
export { default } from './b';
// @filename: d.ts
export default function () { }
export { y as default } from './missing';
export { z as default } from './missing';
// @filename: e.ts
const z = 1;
export default z;
export { y as default } from './c';
// @filename: f.ts
export default function foo(value: number): number
export default function foo(value: string | number): string | number {
    return 1
}
export default interface Bar {}
