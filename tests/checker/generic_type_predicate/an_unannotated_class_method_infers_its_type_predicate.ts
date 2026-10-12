class Inferrer {
  isNumber(x: number | string) {
    return typeof x === 'number';
  }
  isLong(x: string) {
    return x.length > 3;
  }
}
declare let value: number | string;
const inferrer = new Inferrer();
if (inferrer.isNumber(value)) {
  const n: number = value;
} else {
  const s: string = value;
}
if (inferrer.isLong("abcd")) {
  const n: number = value;
}
