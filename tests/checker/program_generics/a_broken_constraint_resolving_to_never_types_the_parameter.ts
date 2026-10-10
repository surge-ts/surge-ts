// tsc puts the constraint in place of an inferred candidate that breaks it
// (`getInferredType`); a conditional constraint over the parameter itself that
// resolves to `never` leaves the argument checked against `never`.
declare function onlyNullable<T extends null extends T ? any : never>(value: T): void;
declare function onlyNullableWrapped<T extends [null] extends [T] ? any : never>(value: T): void;
declare const maybe: string | null;
declare const definite: string;
onlyNullable(maybe);
onlyNullableWrapped(maybe);
onlyNullable(definite);
onlyNullableWrapped(definite);
