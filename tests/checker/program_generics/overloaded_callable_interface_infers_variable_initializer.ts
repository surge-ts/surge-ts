interface BuilderBase {
  _: { name: string };
}
interface Builder<T extends { name: string }> extends BuilderBase {
  _: T;
}
declare function text<TName extends string>(name: TName): Builder<{ name: TName }>;

interface TableFn {
  <TName extends string, TCols extends Record<string, BuilderBase>>(name: TName, cols: TCols): TCols;
  <TName extends string, TCols extends Record<string, BuilderBase>>(name: TName, cols: (t: number) => TCols): TCols;
}
declare const table: TableFn;

const users = table('users', { klass: text('class') });
export const klass: number = users.klass;
const viaCallback = table('posts', () => ({ title: text('title') }));
export const title: number = viaCallback.title;
