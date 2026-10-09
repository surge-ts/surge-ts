declare const pairs: [string, number][];

export const branches: [string, string | number][] = pairs.map(([key, value]) => {
  if (value > 0) {
    return [key, value];
  } else {
    return [key, 'negative'];
  }
});
export const expression: [string, string | number][] = pairs.map(([key, value]) => [key, value]);
export const conditional: [string, string | number][] = pairs.map(([key, value]) => (value > 0 ? [key, value] : [key, 'negative']));
export const readonlyTarget: readonly [string, number][] = pairs.map(([key, value]) => [key, value + 1]);
export const unionTarget: ([string, number] | [number])[] = pairs.map(([key, value]) => (value ? [key, value] : [value]));

const record: Record<string, number> = { a: 1 };
export const swapped: [number, string][] = Object.entries(record).map(([key, value]) => [value, key]);

export const wrongElement: [string, number][] = pairs.map(([key]) => [key, 'x']);
export const plain: string[] = pairs.map(([key]) => key);
