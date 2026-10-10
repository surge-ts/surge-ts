// @strict: true
// @lib: es2023
function* counter(limit: number) {
  for (let i = 0; i < limit; i++) {
    yield i;
  }
  return "done";
}
const gen: number = counter(3);
const step = counter(3).next();
const stepValue: boolean = step.value;
for (const n of counter(2)) {
  const s: string = n;
}

async function* ticks() {
  yield 1;
  yield 2;
}
async function readTicks() {
  for await (const t of ticks()) {
    const s: string = t;
  }
}

function* empty() {}
const emptyNext: number = empty().next().value;

class Bag {
  private items = ["a", "b"];
  *[Symbol.iterator]() {
    yield* this.items;
  }
  *pairs() {
    for (const item of this.items) {
      yield [item, item.length] as const;
    }
  }
}
for (const item of new Bag()) {
  const n: number = item;
}
for (const [name, size] of new Bag().pairs()) {
  const wrongName: number = name;
  const wrongSize: string = size;
}
