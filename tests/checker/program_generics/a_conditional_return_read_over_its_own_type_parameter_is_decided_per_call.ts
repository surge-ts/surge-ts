declare function kind<T>(value: T): T extends string ? "text" : T extends number ? "count" : "other";

function describe<T>(value: T) {
  return kind(value);
}

export const text: "text" = describe("a");
export const count: "count" = describe(1);
export const other: "other" = describe(true);
export const wrong: "text" = describe(1);

function pick<T extends "a" | "b" | 1>(value: T) {
  return kind(value);
}

export const picked: "text" | "count" = pick<"a" | 1>("a");
export const mistaken: "text" = pick<"a" | 1>(1);

declare function shape<T>(value: T): T extends object ? (T extends readonly unknown[] ? "list" : "record") : "scalar";

function classify<T>(value: T) {
  return shape(value);
}

export const list: "list" = classify([1]);
export const record: "record" = classify({ a: 1 });
export const scalar: "scalar" = classify(1);
export const misread: "record" = classify([1]);
