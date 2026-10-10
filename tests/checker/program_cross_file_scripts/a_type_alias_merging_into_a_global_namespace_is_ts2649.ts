// A global `type A` declared after `interface A` and `namespace A` in another
// script meets a namespace symbol it cannot merge with: tsc's merge reports
// TS2649 at the alias, not a duplicate identifier.
// @filename: a_initial.ts
interface A { }
namespace A {}
// @filename: b_final.ts
type A = {}
