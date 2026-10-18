type Receiver = { value: number };
declare const receiver: Receiver;

const method = function (this: Receiver, value: number) { return this.value + value; };
method(1);
method.call(receiver, 1);
const callback = function (this: void, value: number) { return value; };
callback(1);

interface Holder { run: (this: void) => void }
const holder: Holder = { run: function (this: Receiver) {} };
holder.run = function (this: string) {};

const literal = { value: 1, read: function (this: { other: string }) { return 1; } };
literal.read();

function make<T>() { return { method(this: T, value: number) { return value; } }; }
make<number>().method(1);

declare function takes(callback: (this: Receiver, value: number) => void): void;
takes(function (this: Receiver, value) {});
takes(function (this: string, value) {});
export {};
