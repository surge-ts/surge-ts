// An unresolved shorthand target of a destructuring assignment is reported
// as the shorthand property it is written as (TS18004); other targets are
// TS2304.
({ c = 1 } = {});
({ e } = { e: 1 });
({ a: renamed } = { a: 1 });
[missing] = [1];
