declare const pick: ((...args: [string]) => unknown) | ((...args: [number]) => unknown) | ((...args: []) => unknown);
declare const anything: any;
pick(...anything);
pick(1);
declare const sorted: ((...args: []) => unknown) | ((...args: [string]) => unknown) | ((...args: [number]) => unknown);
sorted("x");
sorted(...anything);
declare const lone: (x: never) => void;
lone(...anything);
