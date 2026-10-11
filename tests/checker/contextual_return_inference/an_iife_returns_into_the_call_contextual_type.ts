// @strict: true
// An immediately invoked arrow's returns take the call's contextual type
// (`getContextualReturnType`), so the callbacks they hold are typed by it;
// with no contextual type they are implicitly `any`.
const typed: { serialize: (obj: unknown) => unknown } = (() => {
    return { serialize: (obj) => obj };
})();
const untyped = (() => {
    return { serialize: (obj) => obj };
})();
