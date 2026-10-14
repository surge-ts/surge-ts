// @strict: true
// @lib: es2023,dom
// A member of `PropertyDescriptor` typed `any` gives the method no contextual
// signature, so the literal it returns has no contextual type and its own
// members take the literal as `this`.
Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
  configurable: true,
  value(this: HTMLCanvasElement, kind: string) {
    if (kind !== "2d") return null;
    return {
      font: "",
      measureText(): number {
        return (this.font as string).length + this.width;
      },
    };
  },
});
export {};
