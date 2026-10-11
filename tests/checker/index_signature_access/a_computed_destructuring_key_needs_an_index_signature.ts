// @strict: true
// A destructured property under a computed `string` or `number` key reads
// the parent's index signature (`getPropertyTypeForIndexType`): TS2537 where
// there is none — for any key expression of that type, for a primitive's
// apparent type, and for a written parameter type.
declare const k: string;
declare const n: number;
declare const obj: { a: number };
let target: unknown;

({ [k + ""]: target } = 0);
({ [`${k}`]: target } = obj);
({ [n]: target } = "text");
({ [k]: target } = "text");

function annotated({ [k]: v }: { a: number }, { [k]: w }: any[], { [n]: x }: number[]) {}
