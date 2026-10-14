// @strict: true
// @exactOptionalPropertyTypes: true
// A falsy test of an optional member leaves the slot optional: the object it
// narrows still fits a target whose member is optional without `undefined`.
type Options = { json?: boolean; color?: boolean };
type BuildOptions = Options & { watch?: boolean };
declare function color(o: Pick<Options, "json" | "color">): void;
declare function run(o: Options): void;
export function build(options: BuildOptions) {
  if (options.json) {
    run(options);
  } else {
    run(options);
    color(options);
  }
}
