// @esModuleInterop: true
// @filename: bar.d.ts
import * as foo from './foo'
export as namespace foo
export = foo;

declare global {
    const foo: typeof foo;
}
// @filename: foo.d.ts
interface Root {
    default: Root;
}

declare const root: Root;
export = root;
