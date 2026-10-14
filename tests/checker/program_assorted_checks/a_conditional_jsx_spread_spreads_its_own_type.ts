// @filename: a.tsx
// @jsx: preserve
// @strict: true
// A spread attribute's object literal inside a conditional is spread as its
// own type, not as the props type it was checked against (TS2783 asks which
// properties the spread always writes).
declare namespace JSX {
  interface Element {}
  interface IntrinsicElements { [key: string]: {} }
}
function Button(props: { id: string; to: "a" | "b"; blockedReason?: string }): JSX.Element {
  return props.id as unknown as JSX.Element;
}
declare const blocked: string | undefined;
export const kept = <Button id="1" to="a" {...(blocked ? { blockedReason: blocked } : {})} />;
export const overwritten = <Button id="1" to="a" {...{ id: "2" }} />;
