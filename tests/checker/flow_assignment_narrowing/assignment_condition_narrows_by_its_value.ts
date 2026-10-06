// An assignment tested as a condition narrows by its value first and then by
// its target: `if (thing &&= other)` proves `other` truthy, while `||=` and
// `??=` reach the branch without evaluating `other` when `thing` holds.
interface Thing { name: string }
declare const mode: number;
function check(thing: Thing | undefined, other: Thing | undefined) {
    if (mode === 1) {
        if (thing &&= other) {
            thing.name;
            other.name;
        }
    } else if (mode === 2) {
        if (thing ||= other) {
            thing.name;
            other.name;
        }
    } else if (mode === 3) {
        if (thing ??= other) {
            other.name;
        }
    } else if (mode === 4) {
        if (thing = other) {
            other.name;
        }
    } else {
        if (!(thing &&= other)) {
        } else {
            other.name;
        }
    }
}
