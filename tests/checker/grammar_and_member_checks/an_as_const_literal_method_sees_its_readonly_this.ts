// @noImplicitThis: true
// A method of an `as const` object literal reads `this` as the literal the
// const context types (`getContextualThisParameterType`): its properties are
// read-only.
let o9 = { x: 10, foo() { this.x = 20; } } as const;
let o8 = { x: 10, foo() { this.x = 20; } };
