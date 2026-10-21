// @strict: true
// Assigning an enum member narrows the variable to that member's literal
// type, and an array literal keeps a regular literal it reads (a narrowed
// `let`, a parameter); only a fresh one, like an unannotated const's, widens.
enum Direction { Up, Down }

export function narrowedEnum(): void {
  let direction = Direction.Up;
  direction = Direction.Down;
  if (direction === Direction.Up) {
  }
  const seen = [direction];
  seen.push(Direction.Up);
}

export function narrowedLiteral(): void {
  let step: 1 | 2 = 1;
  step = 2;
  const steps = [step];
  steps.push(1);
}

export function parameterLiteral(kind: "a"): void {
  const kinds = [kind];
  kinds.push("b");
}

export function freshConst(): void {
  const pinned = Direction.Up;
  const pins = [pinned];
  pins.push(Direction.Down);
}
