export type T = { x?: { [Enum.A]: 0 } };
interface I { [Nope.B]: number }
const E = { A: "a" } as const;
type U = { [E.A]: 1 };
