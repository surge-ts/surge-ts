// @strict: true
// @lib: es2023
declare const record: { name: unknown };
const fromMember: number = record.name;
declare const values: unknown[];
const fromElement: number = values[0];
declare const input: unknown;
if (typeof input === "object" && input !== null && "name" in input) {
  const fromPresence: number = input.name;
}
const kept: unknown = record.name;
