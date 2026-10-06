declare let box: { value: { value: typeof box } };
declare var nested: { next: typeof nested; n: number };
const n: number = nested.n;
