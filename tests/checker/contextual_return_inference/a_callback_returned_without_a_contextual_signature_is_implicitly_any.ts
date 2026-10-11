// @strict: true
// @jsx: preserve
// @filename: returns.ts
// A block-bodied arrow or function expression with no annotation and no
// contextual type gives its `return` expressions no contextual type either
// (`getContextualReturnType` is nil), so a callback they hold is implicitly
// `any`, exactly as in a function declaration's body.
export const inArray = () => { return [(value) => value]; };
export const direct = () => { return (value) => {}; };
export const expression = function () { return [(value) => value]; };
export const generic = <T,>(seed: T) => { return (value) => seed; };
export const asynchronous = async () => { return (value) => value; };
export const generator = function* () { return (value) => value; };
export default (x) => { return (y) => 1; };

// A contextual signature still types the returned callback.
export const typed: () => (value: number) => number = () => { return (value) => value; };

// @filename: children.tsx
// The same holds for a render prop returned from an unannotated component,
// when the element offers its children no contextual type.
export type Props =
  | { renderNumber?: false; children: (arg: string) => void }
  | { renderNumber: true; children: (arg: number) => void };

export declare function Foo(props: Props): unknown;

export const Test = () => {
  return <Foo>{(value) => {}}</Foo>;
};
