// @strict: true
// `new` on an `any` value is an untyped call (`resolveUntypedCall`): its
// arguments are still checked, with no contextual type for a callback.
declare var Ctor: any;
new Ctor(missingName);
new Ctor((x) => x);
function construct(p) {
    new p((x) => x);
}
