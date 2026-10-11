// @strict: true
// A receiver the source itself types `any` — written `any`, an annotation
// that does not resolve, or a parameter tsc leaves implicitly `any` — gives a
// call on one of its members no contextual type, so a callback passed to it
// reports its own parameters.
declare var written: any;
written.map((x) => x);
written.nested.map((x) => x);

declare var unresolved: Missing;
unresolved.map((x) => x);

function untyped(p) {
    p.map((x) => x);
}
const arrow = (model) => model.items.map((todo) => todo);

// A parameter the context types is not the source's `any`.
declare function take(callback: (value: any) => void): void;
take((value) => value.map((x: number) => x));
