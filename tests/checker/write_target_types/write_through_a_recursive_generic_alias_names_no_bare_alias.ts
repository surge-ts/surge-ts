// @strict: true
// @lib: es2023
type DeepReadonly<T> = { readonly [K in keyof T]: DeepReadonly<T[K]> };
declare const settings: DeepReadonly<{ theme: { color: string } }>;
settings.theme.color = "red";
