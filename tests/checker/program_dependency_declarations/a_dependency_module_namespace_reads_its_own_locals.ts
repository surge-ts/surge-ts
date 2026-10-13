// @filename: node_modules/dep/package.json
{"name":"dep","version":"1.0.0","types":"index.d.ts"}
// @filename: node_modules/dep/index.d.ts
declare function localInstall(): void
export { localInstall as install }
export default Dep
declare namespace Dep {
  const install: typeof localInstall
}
// @filename: a.ts
import Dep, { install } from "dep";
Dep.install();
install();
