// @strictFunctionTypes: false
declare let wide: (x: {}) => {};
declare let narrow: (x: number) => number;
declare let unrelated: (x: string) => string;
wide = narrow;
narrow = unrelated;
declare function pick<T>(a: (x: T) => T, b: (x: T) => T): (x: T) => T;
const picked: (x: {}) => {} = pick((x: number) => 1, (x: string) => '');
declare function both<T>(value: T, callback: (x: T) => void): T;
const inferred: number = both("a", (x: string | number) => {});
