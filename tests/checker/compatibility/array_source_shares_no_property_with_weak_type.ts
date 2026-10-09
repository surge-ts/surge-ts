// @strict: true
type Weak = { b?: number; name?: string | null };
declare const tuple: [number];
export const fromTuple: Weak = tuple;
declare const numbers: number[];
export const fromArray: Weak = numbers;
export const fromLiteral: Weak = [{ a: 2 }];
type SharesLength = { length?: number; x?: 1 };
export const sharingLength: SharesLength = numbers;
interface Builder {
  values(value: Weak): 1;
  values(values: Weak[]): 2;
}
declare const builder: Builder;
builder.values([{ a: 2 }]);
builder.values([{ name: null }]);
