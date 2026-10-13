// A function type declares no members of its own, so every property an
// object literal writes against it is excess (TS2353 at the property).
const handler: (x: number) => void = { a: 1 };
function register(callback: (x: number) => void) {}
register({ a: 1 });
const empty: () => void = {};
