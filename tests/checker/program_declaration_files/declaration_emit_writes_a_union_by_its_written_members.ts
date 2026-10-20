// @declaration: true
// @emitDeclarationOnly: true
// @filename: errors.ts
interface NoMatch { kind: "no" }
interface MultipleMatch { kind: "many" }
export type Issue = NoMatch | MultipleMatch;
export declare function issue(): Issue | undefined;
export declare function issues(): Array<Issue | null>;

// @filename: index.ts
import { issue, issues } from "./errors";
export const current = issue();
export const all = issues();
