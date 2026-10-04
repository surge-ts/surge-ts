type InvalidConstraint<T> = T extends infer A extends B ? number : string;
type SameClauseInfer<T> = T extends { a: infer U, b: infer V extends U } ? [U, V] : never;
type OutsideTrueBranch<T> = U extends (infer U)[] ? U : U;
type MissingInFalseBranch<T> = T extends (infer A extends string) ? A : Missing;
type Fine<T> = T extends [infer H, ...infer R] ? [H, R] : T extends string ? `${T}!` : never;
