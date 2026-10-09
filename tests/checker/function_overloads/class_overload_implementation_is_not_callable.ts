// @strict: true
type Insert = { id?: number; email: string };
class Builder {
  values(value: Insert): void;
  values(values: Insert[]): void;
  values(_values: unknown): void {}
  static create(name: string): Builder;
  static create(name: string, size: number): Builder;
  static create(_name: unknown, _size?: unknown): Builder {
    return new Builder();
  }
}
declare const builder: Builder;
builder.values(5);
builder.values({ email: 1 });
builder.values({ email: 'a' });
Builder.create(1);
Builder.create('a', 2);
