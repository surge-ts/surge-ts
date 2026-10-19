export function g<T>(x: T) {
  const r = x && "s";
  const narrow: "s" = r;
  return narrow;
}

export function h<T extends { event?: string }>(chunk: T) {
  const error = chunk.event && new Error(chunk.event);
  const kept: Error | null | undefined = error;
  return kept;
}

export function k<T>(x: T) {
  const r = x || "s";
  const wide: "s" = r;
  return wide;
}

export function u<T extends string | undefined>(x: T) {
  const r = x && 1;
  const n: number = r;
  return n;
}

export function w<T extends string | undefined>(x: T): number {
  return x && 1;
}
