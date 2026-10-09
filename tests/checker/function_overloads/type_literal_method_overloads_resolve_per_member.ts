// @strict: true
type Weak = { b?: number; name?: string | null };
type Insert = { id?: number; email: string };
declare const weak: {
  values(value: Weak): 1;
  values(values: Weak[]): 2;
};
export const group: 0 = weak.values;
weak.values({ a: 2 });
declare const insert: {
  values(value: Insert): 1;
  values(values: Insert[]): 2;
};
insert.values({ email: 'a', a: 2 });
insert.values([{ email: 'a', a: 2 }]);
export const one: 1 = insert.values({ email: 'a' });
export const two: 2 = insert.values([{ email: 'a' }]);
export const wrong: 2 = insert.values({ email: 'a' });
