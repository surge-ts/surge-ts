// `new Set()` infers nothing from its arguments (`Set<unknown>`), so the
// default operand of `??` and `||` takes `T` from the contextual reference,
// a nullable one included.
interface Bag { patterns?: Set<string> }
declare const fallback: Set<string> | undefined;
const coalesced: Set<string> = fallback ?? new Set();
const ored: Set<string> = fallback || new Set();

function fill(bag: Bag): number {
  bag.patterns ??= new Set();
  bag.patterns.add("p");
  return bag.patterns.size;
}

function assignDefault(bag: Bag): number {
  bag.patterns = fallback ?? new Set();
  const size: string = bag.patterns.size;
  return size.length;
}
