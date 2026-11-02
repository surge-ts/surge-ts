// @surge-compare: messages
// A property typed by a signature alias relates through the alias's
// arguments before its signatures, read before or after its declaration.
export {};
type Box<out T> = { f: Fn<T> };
type Fn<T> = (x: T) => void;
declare let narrow: { f: Fn<"a"> };
declare let wide: { f: Fn<string> };
wide = narrow;
function local<T>(value: T) {
  type Narrow = { f: Fn<"a">; value: T };
  let narrowed!: Narrow;
  let widened: { f: Fn<string>; value: T } = narrowed;
  return [widened, value];
}
