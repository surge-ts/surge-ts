// @strict: true
// tsc narrows `issue.options` as the union of its members' types, which the
// index signature's `unknown` absorbs: a truthy test leaves `{}` and
// `Array.isArray` `any[]`, so the elements are `any`, not `string | symbol`.
type Flatten<T> = { [k in keyof T]: T[k] } & {};
type WithOptions = Flatten<{ code: "u"; options?: (string | symbol)[] } & Record<string, unknown>>;
type WithErrors = Flatten<{ code: "u"; errors: [] } & Record<string, unknown>>;
declare const issue: WithOptions | WithErrors;
if (issue.options && Array.isArray(issue.options)) {
    issue.options.map((o) => `${o}`);
}
if (typeof issue.options === "string") {
    const n: number = issue.options;
}
