type T08 = [...string];
declare let y: [...number];
type T12 = [string, ...(string | number)];
type T13 = [...any];
type T14 = [...string[]];
type T15<U extends unknown[]> = [...U];
type T16 = [...("a" | "b")];
