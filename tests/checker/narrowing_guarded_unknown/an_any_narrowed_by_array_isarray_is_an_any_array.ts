// @strict: true
// `Array.isArray` narrows an `any` to `any[]` (`getNarrowedType` hands an
// `any` the predicate's type), so a callback passed to one of its methods is
// contextually typed there; the other branch keeps `any`.
function narrowed(p: any) {
    if (Array.isArray(p)) {
        const n: number = p;
        p.every((x, i) => x);
    } else {
        p.map((y) => y);
    }
}
