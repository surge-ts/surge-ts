// @strict: true
// @lib: es2023
export const pick = () => pick();
const nest = () => [nest()];
const count = (n: number) => (n > 0 ? count(n - 1) : 0);
const plusOne = (n: number) => {
  if (n) return 1;
  return plusOne(n - 1) + 1;
};

const walk = (n: number) => {
  if (n < 0) return "done";
  return walk(n - 1);
};
const walked: number = walk(1);
const viaExpression = function (n: number) {
  if (n) return 1;
  return viaExpression(n - 1);
};
const expressionResult: string = viaExpression(1);
const named = function self(n: number) {
  if (n) return 1;
  return self(n - 1);
};
const namedResult: string = named(1);
const forever = () => {
  return forever();
};
const foreverType: number = forever;

export let exported = (n: number) => {
  if (n) return 1;
  return exported(n - 1);
};
let reassigned = (n: number) => {
  if (n) return 1;
  return reassigned(n - 1);
};
reassigned = () => 2;
var hoisted = (n: number) => {
  if (n) return 1;
  return hoisted(n - 1);
};

const shadowed = (shadowed: () => string) => {
  return shadowed();
};
const shadowedResult: number = shadowed(() => "a");
