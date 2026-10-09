// @strict: true
// @lib: es2023
interface Config { enumValues: [string, ...string[]] | undefined }
function fromIndexed<T extends Config>(values: T["enumValues"]) {
  return values!.map(value => value.length);
}
function fromVariable<T extends string[] | undefined>(values: T) {
  const lengths: string[] = values!.map(value => value.length);
}
