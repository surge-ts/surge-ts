// @verbatimModuleSyntax: true
// @module: esnext
// @moduleResolution: bundler
// @filename: main5.ts
export default class C {}
// @filename: main6.ts
interface I {}
export default I;
// @filename: main7.ts
import type C from "./main5";
export default C;
// @filename: main8.ts
class D {}
interface D {}
export default D;
