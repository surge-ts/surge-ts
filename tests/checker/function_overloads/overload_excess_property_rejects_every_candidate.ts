// @strict: true
type Insert = { id?: number; email: string };
interface Builder {
  values(value: Insert): void;
  values(values: Insert[]): void;
}
declare const builder: Builder;
builder.values({ email: 'a', generated: 'x' });
builder.values([{ email: 'a' }, { email: 'b', generated: 'x' }]);
builder.values({
  email: 'a',
  generated: 'x',
});
builder.values({ email: 'a' });
builder.values([{ email: 'a' }]);
