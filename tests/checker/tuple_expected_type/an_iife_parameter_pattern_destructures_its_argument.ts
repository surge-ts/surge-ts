// @strict: true
// An immediately invoked arrow's parameter is typed by its argument (tsc's
// `getContextuallyTypedParameterType`), `undefined` included, so a pattern
// over it reports what destructuring `undefined` reports.
(({}) => 0)(undefined);
(([]) => 0)(undefined);
(({ a }: { a?: number } = {}) => a)(undefined);
