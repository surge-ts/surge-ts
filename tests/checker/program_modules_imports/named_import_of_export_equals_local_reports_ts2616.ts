// @module: commonjs
// @esModuleInterop: true
// @filename: a.ts
class Foo {}
export = Foo;
// @filename: b.ts
import { Foo } from "./a";
