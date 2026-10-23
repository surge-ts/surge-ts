// @strict: true
// With no contextual return type, a block-bodied arrow's returned object
// literals are widened member by member, as a function declaration's are:
// `{ label: string }`, not `{ label: "on" } | { label: "off" }`.
const describe = (flag: boolean) => {
  if (flag) {
    return { label: 'on' };
  }
  return { label: 'off' };
};
export const same = describe(true).label === "x";
export const label: number = describe(true).label;

const single = () => { return { label: 'on', count: 1 }; };
export const same2 = single().label === "x";
export const count: string = single().count;

declare function run<T>(task: () => T): T;
export const ran = run(() => { return { kind: 'a' }; }).kind === "b";

// A contextual return type keeps the literals it names.
export const kept: () => { label: "on" } = () => { return { label: 'on' }; };
