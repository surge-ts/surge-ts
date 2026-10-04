const first: () => Iterator<number> = function* () { yield "a"; };
const second: () => Iterator<number> = function* () { yield "a"; };
const third: () => Iterator<number> = function* () { yield true; };
const fourth: () => AsyncIterable<number> = async function* () { yield "a"; };
const fifth: () => AsyncIterableIterator<number> = async function* () { yield* ["a", "b"]; };
