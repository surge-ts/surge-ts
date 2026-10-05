// @target: esnext
declare function call<TS extends unknown[]>(handler: (...args: TS) => void, ...args: TS): void;
call((x: number, y: number) => x + y);
call((x: number, y: number) => x + y, 1, 2);
call((x: number, y: number) => x + y, 1, 2, 3);
call((x: number, y?: number) => x);
call((x: number, y?: number) => x, 1);
call((x?: number) => x);
call((x: number = 1) => x);
call(() => 0, 1);
call(() => 0);
call((x: number, y: void) => x, 4);
call((x: void, y: void) => 42);
call((x: number | void, y: number | void) => 42, 4);
call((x: void, y: number) => 42);
