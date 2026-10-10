// @module: esnext
// `import defer from from "m"` defers a default import named `from`, which a
// deferred import cannot bind (TS18058); `import defer from "m"` binds `defer`.
// @filename: a.ts
export default 2;
// @filename: b.ts
import defer from from "./a.js";
import defer from "./a.js";
export {};
