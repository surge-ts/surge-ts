// @strict: true
// @lib: es2023
interface Events {
  click: { x: number };
  key: { code: string };
}
interface Emitter {
  on<K extends keyof Events>(type: K, listener: (this: Emitter, event: Events[K]) => any): void;
  on(type: string, listener: (event: unknown) => any): void;
}
declare const emitter: Emitter;
emitter.on("click", (event) => {
  const wrongX: string = event.x;
});
emitter.on("key", (event) => {
  const wrongCode: number = event.code;
});
emitter.on("other", (event) => {});
