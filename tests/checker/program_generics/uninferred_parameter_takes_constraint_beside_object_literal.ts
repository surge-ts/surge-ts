type Writable<T> = { -readonly [P in keyof T]: T[P] };

interface Config<
  TEnum extends readonly string[] | undefined = readonly string[] | undefined,
  TLength extends number | undefined = number | undefined,
> {
  enum?: TEnum;
  length?: TLength;
}

declare function column<TName extends string, U extends string, T extends Readonly<[U, ...U[]]>, L extends number | undefined>(
  name: TName,
  config?: Config<T | Writable<T>, L>,
): { name: TName; values: Writable<T>; length: L };

const sized = column('a', { length: 255 });
const enumerated = column('b', { enum: ['x', 'y'] });

export const values: [string, ...string[]] = sized.values;
export const length: number | undefined = enumerated.length;
export const wrongValues: number = sized.values;
export const wrongLength: string = enumerated.length;
