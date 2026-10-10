// A JSDoc `T?` is `T | null`, which no rest element may spread (TS2574).
type T08 = [...string];
type T09 = [...string?];
type T10 = [...(number | boolean)?];
