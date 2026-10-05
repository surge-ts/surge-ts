// @module: esnext
// @target: esnext
// @lib: es5,dom
export {};
const a = import.meta.scriptElement;
const u: number = import.meta.url;
const m: string = import.meta;
import.meta.blah = 1;
declare global { interface ImportMeta { foo?: () => void } }
if (import.meta.foo) {
    import.meta.foo();
}
