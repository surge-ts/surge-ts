interface Prepared<T> { run(placeholderValues?: Record<string, unknown>): T; all(v?: number): T[] }
export class Q {
  prepare(): Prepared<string> { return null!; }
  run: ReturnType<this['prepare']>['run'] = (placeholderValues) => {
    const p: 1 = placeholderValues;
    return "";
  };
  all: ReturnType<Q['prepare']>['all'] = (v) => [];
  x: this['prepare'] = () => null!;
}
