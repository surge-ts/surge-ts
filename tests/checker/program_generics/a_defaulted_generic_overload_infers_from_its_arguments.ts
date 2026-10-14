// @strict: true
// An overloaded generic member resolved with its type parameter's default
// (`Object.fromEntries<T = any>`) still infers from the arguments where a
// literal's member reads its type, and a contextual return still decides it
// where there is one.
type NoteChoice = "" | "true" | "false";
declare function initialNote(r: { code: string }): NoteChoice;
declare const added: { code: string }[];
const holder = { notes: Object.fromEntries(added.map((r) => [r.code, initialNote(r)])) };
export const wrong: number = holder.notes["a"];
const listed = [Object.fromEntries(added.map((r) => [r.code, 1]))];
export const count: string = listed[0]!["a"];
type A = "x" | "y";
export const set: ReadonlySet<A> = new Set(["x", "y"]);
